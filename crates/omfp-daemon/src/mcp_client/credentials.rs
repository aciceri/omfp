//! omp's MCP OAuth credentials in its `agent.db` (DMN-352, DMN-353): the daemon reads the login omp made for a
//! server and refreshes it the way omp 19.1.0 does (`mcp/oauth-credentials.ts`, `ai/src/auth/refresh.ts`): under the
//! row's lease in `auth_credential_refresh_leases`, written back by compare-and-set, so a thread's omp and the daemon
//! never spend each other's rotated refresh tokens. The database's own triggers bump `auth_change_revision`, which
//! tells running omp processes the row changed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::{Client, Url};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Map, Value};
use tracing::warn;

use crate::omp_mcp::{Auth, Remote};
use crate::store::{new_id, now_ms};

/// omp refreshes an MCP token that expires within five minutes (`refreshStoredManagedMcpOAuthCredential`).
const REFRESH_SKEW_MS: i64 = 5 * 60_000;
/// omp's `OAUTH_REFRESH_LEASE_TTL_MS`.
const LEASE_TTL_MS: i64 = 15_000;
/// omp's lease polling bounds (`OAUTH_REFRESH_LEASE_POLL_MS`, `OAUTH_REFRESH_LEASE_WAIT_MAX_MS`).
const LEASE_POLL_MS: i64 = 50;
const LEASE_WAIT_MAX_MS: i64 = 250;
/// omp's `OAUTH_REFRESH_OPERATION_TIMEOUT_MS`: waiting on a peer's lease and the exchange share it.
const REFRESH_BUDGET_MS: i64 = 10_000;
/// omp's `REMOTE_REFRESH_SENTINEL`: an auth broker holds the refresh token.
const REMOTE_REFRESH: &str = "__remote__";
/// The omp profile of omfp's isolated omp (ARC-021): omp's `getActiveProfile()` without `--profile`.
const PROFILE: &str = "default";

/// The access token of one stored credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Token {
    /// The row's `data` as read: the compare-and-set value, and how a peer's newer token is recognized.
    data: String,
    pub access: String,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum CredentialError {
    #[error("cannot read omp's credential store: {0}")]
    Store(#[from] rusqlite::Error),
    #[error("the token refresh failed: {0}")]
    Refresh(String),
    #[error("internal error: {0}")]
    Internal(String),
}

type Result<T, E = CredentialError> = std::result::Result<T, E>;

/// omp's credential store, `<data-dir>/omp-profile/agent/agent.db` (ARC-021).
#[derive(Debug, Clone)]
pub(super) struct Credentials {
    db: PathBuf,
}

/// The credential ids (`provider` column) omp tries for `remote`, in order (`lookupMcpOAuthCredential`): the
/// `auth.credentialId` of the entry (a profile-scoped one only when it names this URL), then, unless the entry sets its
/// own `Authorization` header, the id omp keys by the server URL.
fn credential_ids(remote: &Remote, explicit_only: bool) -> Vec<String> {
    let keyed = format!("mcp_oauth:profile:{PROFILE}:{}", remote.url);
    let mut ids = Vec::new();
    if let Some(id) = remote
        .auth
        .as_ref()
        .and_then(|auth| auth.credential_id.as_ref())
        && (!id.starts_with("mcp_oauth:profile:") || *id == keyed)
    {
        ids.push(id.clone());
    }
    if !explicit_only && !ids.contains(&keyed) {
        ids.push(keyed);
    }
    ids
}

struct Row {
    id: i64,
    data: String,
}

fn open(db: &Path) -> Result<Option<Connection>> {
    if !db.is_file() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(Some(conn))
}

/// The first active OAuth row of the first id that has one, as omp's `credentials.get` picks it.
fn read_row(conn: &Connection, ids: &[String]) -> Result<Option<Row>> {
    let mut statement = conn.prepare_cached(
        "SELECT id, data FROM auth_credentials \
         WHERE provider = ?1 AND credential_type = 'oauth' AND disabled_cause IS NULL ORDER BY id ASC LIMIT 1",
    )?;
    for id in ids {
        let row = statement
            .query_row([id], |row| {
                Ok(Row {
                    id: row.get(0)?,
                    data: row.get(1)?,
                })
            })
            .optional()?;
        if row.is_some() {
            return Ok(row);
        }
    }
    Ok(None)
}

fn read_by_id(conn: &Connection, id: i64) -> Result<Option<Row>> {
    Ok(conn
        .query_row(
            "SELECT id, data FROM auth_credentials WHERE id = ?1 AND disabled_cause IS NULL",
            [id],
            |row| {
                Ok(Row {
                    id: row.get(0)?,
                    data: row.get(1)?,
                })
            },
        )
        .optional()?)
}

/// omp's `tryAcquireCredentialRefreshLease`: takes the row's lease unless a peer holds one that has not expired.
fn acquire_lease(conn: &Connection, row: i64, owner: &str) -> Result<bool> {
    let now = now_ms();
    let changed = conn.execute(
        "INSERT INTO auth_credential_refresh_leases (credential_id, owner, expires_at_ms, updated_at) \
         VALUES (?1, ?2, ?3, CAST(strftime('%s','now') AS INTEGER)) \
         ON CONFLICT(credential_id) DO UPDATE SET owner = excluded.owner, expires_at_ms = excluded.expires_at_ms, \
         updated_at = excluded.updated_at WHERE auth_credential_refresh_leases.expires_at_ms <= ?4",
        params![row, owner, now + LEASE_TTL_MS, now],
    )?;
    Ok(changed == 1)
}

fn lease_expires_at(conn: &Connection, row: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT expires_at_ms FROM auth_credential_refresh_leases WHERE credential_id = ?1",
            [row],
            |row| row.get(0),
        )
        .optional()?)
}

fn release_lease(conn: &Connection, row: i64, owner: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM auth_credential_refresh_leases WHERE credential_id = ?1 AND owner = ?2",
        params![row, owner],
    )?;
    Ok(())
}

/// omp's `tryUpdateAuthCredentialIfMatches` with its lease fence: writes `data` only over the row as read, while this
/// owner still holds the lease.
fn save(conn: &Connection, row: i64, expected: &str, data: &str, owner: &str) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE auth_credentials SET data = ?1, updated_at = CAST(strftime('%s','now') AS INTEGER) \
         WHERE id = ?2 AND data = ?3 AND disabled_cause IS NULL AND EXISTS ( \
           SELECT 1 FROM auth_credential_refresh_leases WHERE credential_id = ?2 AND owner = ?4 AND expires_at_ms > ?5)",
        params![data, row, expected, owner, now_ms()],
    )?;
    Ok(changed == 1)
}

/// A stored OAuth credential's fields the daemon reads.
struct Stored {
    fields: Map<String, Value>,
    access: String,
    expires: i64,
}

fn parse(data: &str) -> Option<Stored> {
    let Value::Object(fields) = serde_json::from_str(data).ok()? else {
        return None;
    };
    let access = fields.get("access")?.as_str()?.to_owned();
    let expires = fields.get("expires").and_then(Value::as_f64).unwrap_or(0.0);
    #[allow(clippy::cast_possible_truncation)] // Unix milliseconds.
    let expires = expires as i64;
    Some(Stored {
        fields,
        access,
        expires,
    })
}

fn text(fields: &Map<String, Value>, key: &str) -> Option<String> {
    fields
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
}

/// Where a refresh goes (omp's `selectMcpOAuthRefreshMaterial`): the credential's own fields when it carries a
/// `tokenUrl`, else the entry's `auth` block (credentials minted before omp stored them); the scope the grant holds
/// always comes from the credential (omp 19.1, `refreshManagedMcpOAuthCredential`).
struct Material {
    token_url: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    resource: Option<String>,
    authorization_url: Option<String>,
    from_credential: bool,
    scope: Option<String>,
}

fn material(fields: &Map<String, Value>, auth: Option<&Auth>) -> Option<Material> {
    if let Some(token_url) = text(fields, "tokenUrl") {
        return Some(Material {
            token_url,
            client_id: text(fields, "clientId"),
            client_secret: text(fields, "clientSecret"),
            resource: text(fields, "resource"),
            authorization_url: text(fields, "authorizationUrl"),
            from_credential: true,
            scope: text(fields, "scope"),
        });
    }
    let auth = auth?;
    Some(Material {
        token_url: auth.token_url.clone()?,
        client_id: auth.client_id.clone(),
        client_secret: auth.client_secret.clone(),
        resource: auth.resource.clone(),
        authorization_url: None,
        from_credential: false,
        scope: text(fields, "scope"),
    })
}

fn same_origin(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

/// omp's `refreshMCPOAuthToken`: the `refresh_token` grant, with the server URL as the fallback resource indicator
/// unless it is on the authorization server's origin, and the scope the grant holds (Microsoft Entra refuses a refresh
/// naming none when the client is also the resource).
async fn exchange(
    http: &Client,
    material: &Material,
    refresh: &str,
    server_url: &str,
) -> Result<(String, Option<String>, i64)> {
    let mut form: Vec<(&str, &str)> =
        vec![("grant_type", "refresh_token"), ("refresh_token", refresh)];
    if let Some(client_id) = material.client_id.as_deref().map(str::trim) {
        form.push(("client_id", client_id));
    }
    let fallback = material.resource.is_none();
    let anchor = material
        .authorization_url
        .as_deref()
        .unwrap_or(&material.token_url);
    let resource = material.resource.as_deref().unwrap_or(server_url);
    if !(fallback && same_origin(resource, anchor)) {
        form.push(("resource", resource));
    }
    if let Some(secret) = material.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    if let Some(scope) = material.scope.as_deref().map(str::trim) {
        form.push(("scope", scope));
    }
    let response = http
        .post(&material.token_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&form)
        .timeout(Duration::from_millis(REFRESH_BUDGET_MS.unsigned_abs()))
        .send()
        .await
        .map_err(|error| CredentialError::Refresh(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body: String = body.chars().take(300).collect();
        return Err(CredentialError::Refresh(format!("HTTP {status}: {body}")));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|error| CredentialError::Refresh(error.to_string()))?;
    let access = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| CredentialError::Refresh("the answer has no access_token".into()))?;
    let rotated = body.get("refresh_token").and_then(Value::as_str);
    let expires_in = body
        .get("expires_in")
        .and_then(Value::as_f64)
        .unwrap_or(3600.0);
    #[allow(clippy::cast_possible_truncation)] // Seconds to Unix milliseconds.
    let expires = now_ms().saturating_add((expires_in * 1000.0) as i64);
    Ok((access.to_owned(), rotated.map(str::to_owned), expires))
}

/// omp's `mergeRefreshedCredential`: the stored fields with the new token, and the refresh material written back.
fn merged(
    mut fields: Map<String, Value>,
    material: &Material,
    access: String,
    refresh: String,
    expires: i64,
) -> String {
    fields.insert("access".into(), Value::String(access));
    fields.insert("refresh".into(), Value::String(refresh));
    fields.insert("expires".into(), Value::from(expires));
    if !material.from_credential {
        let mut set = |key: &str, value: Option<&String>| match value {
            Some(value) => {
                fields.insert(key.to_owned(), Value::String(value.clone()));
            }
            None => {
                fields.remove(key);
            }
        };
        set("tokenUrl", Some(&material.token_url));
        set("clientId", material.client_id.as_ref());
        set("clientSecret", material.client_secret.as_ref());
        set("resource", material.resource.as_ref());
        set("authorizationUrl", None);
    }
    Value::Object(fields).to_string()
}

/// What one look at the row decided.
enum Step {
    Use(Token),
    None,
    /// A peer holds the lease: look again after this many milliseconds.
    Wait(i64),
    /// This daemon took the lease: exchange the refresh token of this row.
    Refresh(Box<Refreshing>),
}

/// The row whose lease this daemon took, and how to refresh it.
struct Refreshing {
    row: Row,
    stored: Stored,
    material: Material,
    refresh: String,
}

impl Credentials {
    pub(super) fn new(db: PathBuf) -> Self {
        Self { db }
    }

    async fn on_db<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
        missing: T,
    ) -> Result<T> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || match open(&db)? {
            Some(conn) => work(&conn),
            None => Ok(missing),
        })
        .await
        .map_err(|error| CredentialError::Internal(error.to_string()))?
    }

    /// The access token omp stored for `remote`, refreshed first when it expires within five minutes, or, with
    /// `force` (the server refused it), unless a peer already stored a newer one than `observed` (DMN-352, DMN-353).
    /// `None` without a stored OAuth credential. A failed refresh keeps the stored token, as omp does, except a
    /// forced one, which fails; the daemon never disables omp's row.
    ///
    /// The lookup, the lease, the exchange and its save run in a task of their own: a caller that stops waiting (an
    /// MCP operation's timeout) never drops a rotated refresh token the provider already handed out, nor a held lease.
    pub(super) async fn token(
        &self,
        http: &Client,
        remote: &Remote,
        explicit_only: bool,
        observed: Option<&Token>,
        force: bool,
    ) -> Result<Option<Token>> {
        if remote.auth.as_ref().is_some_and(|auth| !auth.oauth) {
            return Ok(None);
        }
        let (this, http, remote) = (self.clone(), http.clone(), remote.clone());
        let observed = observed.map(|token| token.data.clone());
        tokio::spawn(async move {
            this.lookup(&http, &remote, explicit_only, observed, force)
                .await
        })
        .await
        .map_err(|error| CredentialError::Internal(error.to_string()))?
    }

    async fn lookup(
        &self,
        http: &Client,
        remote: &Remote,
        explicit_only: bool,
        observed: Option<String>,
        force: bool,
    ) -> Result<Option<Token>> {
        let ids = credential_ids(remote, explicit_only);
        let owner = format!("omfp-{}", new_id());
        let auth = remote.auth.clone();
        let deadline = now_ms() + REFRESH_BUDGET_MS;
        loop {
            let (ids, lease_owner, auth, observed) =
                (ids.clone(), owner.clone(), auth.clone(), observed.clone());
            let step = self
                .on_db(
                    move |conn| {
                        look(
                            conn,
                            &ids,
                            &lease_owner,
                            auth.as_ref(),
                            observed.as_deref(),
                            force,
                        )
                    },
                    Step::None,
                )
                .await?;
            match step {
                Step::Use(token) => return Ok(Some(token)),
                Step::None => return Ok(None),
                Step::Wait(ms) => {
                    let left = deadline - now_ms();
                    if left <= 0 {
                        return Err(CredentialError::Refresh(
                            "another process kept the token's refresh lease".into(),
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(ms.min(left).unsigned_abs())).await;
                }
                Step::Refresh(refreshing) => {
                    return self.refresh(http, remote, &owner, *refreshing, force).await;
                }
            }
        }
    }

    /// Exchanges the refresh token under the lease this daemon holds, stores the result and gives the lease back.
    async fn refresh(
        &self,
        http: &Client,
        remote: &Remote,
        owner: &str,
        refreshing: Refreshing,
        force: bool,
    ) -> Result<Option<Token>> {
        let Refreshing {
            row,
            stored,
            material,
            refresh,
        } = refreshing;
        let current = Token {
            data: row.data.clone(),
            access: stored.access.clone(),
        };
        let material = &material;
        let exchanged = exchange(http, material, &refresh, &remote.url).await;
        let owner = owner.to_owned();
        let (access, rotated, expires) = match exchanged {
            Ok(exchanged) => exchanged,
            Err(error) => {
                let id = row.id;
                self.on_db(move |conn| release_lease(conn, id, &owner), ())
                    .await?;
                warn!(server = %remote.name, %error, "the MCP token refresh failed");
                return if force { Err(error) } else { Ok(Some(current)) };
            }
        };
        let data = merged(
            stored.fields,
            material,
            access.clone(),
            rotated.unwrap_or(refresh),
            expires,
        );
        let (id, expected) = (row.id, row.data);
        self.on_db(
            move |conn| {
                let saved = save(conn, id, &expected, &data, &owner)?;
                release_lease(conn, id, &owner)?;
                if saved {
                    return Ok(Some(Token { data, access }));
                }
                // A peer that took a lapsed lease saved first: its row is the one to use.
                warn!("the refreshed MCP token was not saved: another process changed the row");
                Ok(read_by_id(conn, id)?.and_then(|row| {
                    parse(&row.data).map(|stored| Token {
                        data: row.data,
                        access: stored.access,
                    })
                }))
            },
            None,
        )
        .await
    }
}

/// One look at the row (omp's `#refreshStored` loop): use its token, wait for a peer's lease, or take the lease.
fn look(
    conn: &Connection,
    ids: &[String],
    owner: &str,
    auth: Option<&Auth>,
    observed: Option<&str>,
    force: bool,
) -> Result<Step> {
    let Some(row) = read_row(conn, ids)? else {
        return Ok(Step::None);
    };
    let Some(stored) = parse(&row.data) else {
        return Ok(Step::None);
    };
    let token = Token {
        data: row.data.clone(),
        access: stored.access.clone(),
    };
    let fresh = now_ms() + REFRESH_SKEW_MS < stored.expires;
    let newer = observed.is_some_and(|observed| observed != row.data);
    if fresh && (newer || !force) {
        return Ok(Step::Use(token));
    }
    let refresh = text(&stored.fields, "refresh").filter(|refresh| refresh != REMOTE_REFRESH);
    let (Some(refresh), Some(material)) = (refresh, material(&stored.fields, auth)) else {
        return Ok(Step::Use(token));
    };
    if acquire_lease(conn, row.id, owner)? {
        // The lease's holder may have just rotated the row: look again under the lease.
        let Some(again) = read_by_id(conn, row.id)? else {
            release_lease(conn, row.id, owner)?;
            return Ok(Step::None);
        };
        if again.data != row.data {
            release_lease(conn, row.id, owner)?;
            return Ok(Step::Wait(0));
        }
        return Ok(Step::Refresh(Box::new(Refreshing {
            row,
            stored,
            material,
            refresh,
        })));
    }
    let wait = lease_expires_at(conn, row.id)?.map_or(LEASE_POLL_MS, |at| {
        (at - now_ms()).clamp(LEASE_POLL_MS, LEASE_WAIT_MAX_MS)
    });
    Ok(Step::Wait(wait))
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::routing::post;
    use axum::{Form, Json, Router};
    use omfp_protocol::McpTransport;
    use serde_json::json;

    use super::*;

    /// The tables of omp 19.1.0's `agent.db` the daemon touches, with omp's revision trigger on credential updates.
    pub(in crate::mcp_client) const SCHEMA: &str = "
        CREATE TABLE auth_credentials (
            id INTEGER PRIMARY KEY AUTOINCREMENT, provider TEXT NOT NULL, credential_type TEXT NOT NULL,
            data TEXT NOT NULL, disabled_cause TEXT DEFAULT NULL, identity_key TEXT DEFAULT NULL,
            created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER)),
            updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER)));
        CREATE TABLE auth_credential_refresh_leases (
            credential_id INTEGER PRIMARY KEY, owner TEXT NOT NULL, expires_at_ms INTEGER NOT NULL,
            updated_at INTEGER NOT NULL);
        CREATE TABLE auth_change_revision (id INTEGER PRIMARY KEY CHECK (id = 1), revision INTEGER NOT NULL);
        INSERT INTO auth_change_revision (id, revision) VALUES (1, 0);
        CREATE TRIGGER auth_change_revision_auth_credentials_update AFTER UPDATE ON auth_credentials
        BEGIN UPDATE auth_change_revision SET revision = revision + 1 WHERE id = 1; END;
    ";

    struct Fixture {
        _dir: tempfile::TempDir,
        db: PathBuf,
        credentials: Credentials,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("agent.db");
        Connection::open(&db)
            .unwrap()
            .execute_batch(SCHEMA)
            .unwrap();
        Fixture {
            credentials: Credentials::new(db.clone()),
            db,
            _dir: dir,
        }
    }

    impl Fixture {
        fn conn(&self) -> Connection {
            Connection::open(&self.db).unwrap()
        }

        fn insert(&self, provider: &str, data: &Value) -> i64 {
            let conn = self.conn();
            conn.execute(
                "INSERT INTO auth_credentials (provider, credential_type, data) VALUES (?1, 'oauth', ?2)",
                params![provider, data.to_string()],
            )
            .unwrap();
            conn.last_insert_rowid()
        }

        fn data(&self, id: i64) -> Value {
            let data: String = self
                .conn()
                .query_row(
                    "SELECT data FROM auth_credentials WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            serde_json::from_str(&data).unwrap()
        }

        fn revision(&self) -> i64 {
            self.conn()
                .query_row("SELECT revision FROM auth_change_revision", [], |row| {
                    row.get(0)
                })
                .unwrap()
        }

        fn leases(&self) -> i64 {
            self.conn()
                .query_row(
                    "SELECT COUNT(*) FROM auth_credential_refresh_leases",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        }
    }

    /// The form fields of one token request, in order.
    type Fields = Vec<(String, String)>;

    /// A token endpoint answering each refresh with `access-<n>`/`refresh-<n>`, or the given failure.
    #[derive(Clone, Default)]
    struct TokenServer {
        forms: Arc<Mutex<Vec<Fields>>>,
        fail: Arc<Mutex<bool>>,
    }

    async fn token_endpoint(
        State(server): State<TokenServer>,
        Form(form): Form<Vec<(String, String)>>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        let n = {
            let mut forms = server.forms.lock().unwrap();
            forms.push(form);
            forms.len()
        };
        if *server.fail.lock().unwrap() {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid_grant" })),
            )
                .into_response();
        }
        Json(json!({ "access_token": format!("access-{n}"), "refresh_token": format!("refresh-{n}"), "expires_in": 600 }))
            .into_response()
    }

    async fn token_server() -> (TokenServer, String) {
        let server = TokenServer::default();
        let app = Router::new()
            .route("/token", post(token_endpoint))
            .with_state(server.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (server, format!("http://{address}/token"))
    }

    fn remote(url: &str, auth: Option<Auth>) -> Remote {
        Remote {
            name: "linear".into(),
            transport: McpTransport::Http,
            url: url.into(),
            headers: Vec::new(),
            literal_headers: false,
            auth,
            timeout: None,
        }
    }

    const URL: &str = "https://mcp.linear.example/mcp";

    fn keyed() -> String {
        format!("mcp_oauth:profile:default:{URL}")
    }

    fn stored(token_url: &str, expires: i64) -> Value {
        json!({
            "access": "access-0", "refresh": "refresh-0", "expires": expires, "tokenUrl": token_url,
            "clientId": "client", "authorizationUrl": "https://auth.linear.example/authorize"
        })
    }

    // spec: DMN-352
    #[tokio::test]
    async fn a_fresh_token_is_used_as_omp_stored_it() {
        let f = fixture();
        let (server, token_url) = token_server().await;
        f.insert(&keyed(), &stored(&token_url, now_ms() + 3_600_000));

        let token = f
            .credentials
            .token(&Client::new(), &remote(URL, None), false, None, false)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(token.access, "access-0");
        assert_eq!(server.forms.lock().unwrap().len(), 0);
        let none = f
            .credentials
            .token(
                &Client::new(),
                &remote("https://other.example/mcp", None),
                false,
                None,
                false,
            )
            .await
            .unwrap();
        assert_eq!(none, None);
    }

    // spec: DMN-352
    #[tokio::test]
    async fn an_authorization_header_limits_the_lookup_to_the_entry_credential_id() {
        let f = fixture();
        f.insert(
            &keyed(),
            &stored("https://auth.example/token", now_ms() + 3_600_000),
        );
        f.insert(
            "mcp_oauth_legacy",
            &json!({ "access": "legacy", "expires": now_ms() + 3_600_000 }),
        );
        let http = Client::new();

        let keyed_only = f
            .credentials
            .token(&http, &remote(URL, None), true, None, false)
            .await
            .unwrap();
        assert_eq!(keyed_only, None);
        let explicit = Auth {
            oauth: true,
            credential_id: Some("mcp_oauth_legacy".into()),
            ..Auth::default()
        };
        let token = f
            .credentials
            .token(&http, &remote(URL, Some(explicit)), true, None, false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(token.access, "legacy");
        let apikey = Auth::default();
        assert_eq!(
            f.credentials
                .token(&http, &remote(URL, Some(apikey)), false, None, false)
                .await
                .unwrap(),
            None
        );
        // No store at all: no credential.
        let missing = Credentials::new(f.db.with_file_name("missing.db"));
        assert_eq!(
            missing
                .token(&http, &remote(URL, None), false, None, false)
                .await
                .unwrap(),
            None
        );
    }

    // spec: DMN-353
    #[tokio::test]
    async fn an_expiring_token_is_refreshed_under_the_lease_and_written_back_for_omp() {
        let f = fixture();
        let (server, token_url) = token_server().await;
        let id = f.insert(&keyed(), &stored(&token_url, now_ms() + 60_000));

        let token = f
            .credentials
            .token(&Client::new(), &remote(URL, None), false, None, false)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(token.access, "access-1");
        let forms = server.forms.lock().unwrap().clone();
        assert_eq!(
            forms,
            vec![vec![
                ("grant_type".to_owned(), "refresh_token".to_owned()),
                ("refresh_token".to_owned(), "refresh-0".to_owned()),
                ("client_id".to_owned(), "client".to_owned()),
                ("resource".to_owned(), URL.to_owned()),
            ]]
        );
        let data = f.data(id);
        assert_eq!(data["access"], "access-1");
        assert_eq!(data["refresh"], "refresh-1");
        assert!(data["expires"].as_i64().unwrap() > now_ms() + 500_000);
        assert_eq!(data["clientId"], "client");
        assert_eq!(
            data["authorizationUrl"],
            "https://auth.linear.example/authorize"
        );
        assert_eq!(f.revision(), 1, "omp's trigger saw the update");
        assert_eq!(f.leases(), 0, "the lease was given back");
    }

    // spec: DMN-353
    #[test]
    fn a_peer_holding_the_lease_is_waited_for_and_its_new_token_used() {
        let f = fixture();
        let id = f.insert(
            &keyed(),
            &stored("https://auth.example/token", now_ms() + 60_000),
        );
        let conn = f.conn();
        conn.execute(
            "INSERT INTO auth_credential_refresh_leases VALUES (?1, 'omp-peer', ?2, 0)",
            params![id, now_ms() + LEASE_TTL_MS],
        )
        .unwrap();
        let ids = [keyed()];

        let waits = look(&conn, &ids, "omfp-a", None, None, false).unwrap();
        assert!(
            matches!(waits, Step::Wait(ms) if (LEASE_POLL_MS..=LEASE_WAIT_MAX_MS).contains(&ms)),
            "the peer's lease is waited for"
        );

        // The peer saves its rotated token and gives the lease back: the next look uses it.
        let rotated = json!({ "access": "peer", "refresh": "peer-r", "expires": now_ms() + 3_600_000, "tokenUrl": "x" });
        conn.execute(
            "UPDATE auth_credentials SET data = ?1 WHERE id = ?2",
            params![rotated.to_string(), id],
        )
        .unwrap();
        conn.execute("DELETE FROM auth_credential_refresh_leases", [])
            .unwrap();
        let used = look(&conn, &ids, "omfp-a", None, None, false).unwrap();
        assert!(matches!(used, Step::Use(token) if token.access == "peer"));
        assert_eq!(f.leases(), 0, "no refresh was started");

        // A lapsed lease of a peer that died is taken over.
        let expiring =
            json!({ "access": "a", "refresh": "r", "expires": now_ms() + 60_000, "tokenUrl": "x" });
        conn.execute(
            "UPDATE auth_credentials SET data = ?1 WHERE id = ?2",
            params![expiring.to_string(), id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO auth_credential_refresh_leases VALUES (?1, 'omp-dead', ?2, 0)",
            params![id, now_ms() - 1],
        )
        .unwrap();
        let taken = look(&conn, &ids, "omfp-a", None, None, false).unwrap();
        assert!(matches!(taken, Step::Refresh(_)));
        let owner: String = conn
            .query_row(
                "SELECT owner FROM auth_credential_refresh_leases",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owner, "omfp-a");
    }

    // spec: DMN-353
    #[tokio::test]
    async fn a_refused_token_is_replaced_by_a_peer_newer_one_or_refreshed() {
        let f = fixture();
        let (server, token_url) = token_server().await;
        let id = f.insert(&keyed(), &stored(&token_url, now_ms() + 3_600_000));
        let http = Client::new();
        let observed = f
            .credentials
            .token(&http, &remote(URL, None), false, None, false)
            .await
            .unwrap();

        // The server refused it: still fresh and nobody stored a newer one, so it is refreshed.
        let forced = f
            .credentials
            .token(&http, &remote(URL, None), false, observed.as_ref(), true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(forced.access, "access-1");

        // A peer stored a newer token since: it is used as is.
        let newer = json!({ "access": "peer", "refresh": "peer-r", "expires": now_ms() + 3_600_000, "tokenUrl": token_url });
        f.conn()
            .execute(
                "UPDATE auth_credentials SET data = ?1 WHERE id = ?2",
                params![newer.to_string(), id],
            )
            .unwrap();
        let adopted = f
            .credentials
            .token(&http, &remote(URL, None), false, Some(&forced), true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(adopted.access, "peer");
        assert_eq!(server.forms.lock().unwrap().len(), 1);
    }

    // spec: DMN-353
    #[tokio::test]
    async fn a_failed_refresh_keeps_omp_row_and_fails_only_when_forced() {
        let f = fixture();
        let (server, token_url) = token_server().await;
        *server.fail.lock().unwrap() = true;
        let id = f.insert(&keyed(), &stored(&token_url, now_ms() + 60_000));
        let http = Client::new();

        let kept = f
            .credentials
            .token(&http, &remote(URL, None), false, None, false)
            .await
            .unwrap();
        assert_eq!(kept.map(|token| token.access), Some("access-0".to_owned()));
        let forced = f
            .credentials
            .token(&http, &remote(URL, None), false, None, true)
            .await;
        assert!(
            matches!(forced, Err(CredentialError::Refresh(_))),
            "{forced:?}"
        );

        let disabled: Option<String> = f
            .conn()
            .query_row(
                "SELECT disabled_cause FROM auth_credentials WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(disabled, None);
        assert_eq!(f.data(id)["refresh"], "refresh-0");
        assert_eq!(f.leases(), 0);
    }

    // spec: DMN-353
    #[tokio::test]
    async fn a_credential_without_refresh_material_uses_the_entry_auth_block() {
        let f = fixture();
        let (server, token_url) = token_server().await;
        let id = f.insert(
            "mcp_oauth_legacy",
            &json!({ "access": "old", "refresh": "r0", "expires": now_ms() - 1 }),
        );
        let auth = Auth {
            oauth: true,
            credential_id: Some("mcp_oauth_legacy".into()),
            token_url: Some(token_url.clone()),
            client_id: Some("legacy-client".into()),
            client_secret: Some("s3cret".into()),
            resource: Some("https://api.linear.example".into()),
        };

        let token = f
            .credentials
            .token(&Client::new(), &remote(URL, Some(auth)), false, None, false)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(token.access, "access-1");
        assert_eq!(
            server.forms.lock().unwrap()[0],
            vec![
                ("grant_type".to_owned(), "refresh_token".to_owned()),
                ("refresh_token".to_owned(), "r0".to_owned()),
                ("client_id".to_owned(), "legacy-client".to_owned()),
                (
                    "resource".to_owned(),
                    "https://api.linear.example".to_owned()
                ),
                ("client_secret".to_owned(), "s3cret".to_owned()),
            ]
        );
        let data = f.data(id);
        assert_eq!(data["tokenUrl"], token_url);
        assert_eq!(data["clientId"], "legacy-client");
        assert_eq!(data["resource"], "https://api.linear.example");
    }

    // spec: DMN-353
    #[tokio::test]
    async fn the_fallback_resource_is_left_out_on_the_authorization_server_origin() {
        let f = fixture();
        let (server, token_url) = token_server().await;
        let same_origin_server = token_url.replace("/token", "/mcp");
        let provider = format!("mcp_oauth:profile:default:{same_origin_server}");
        f.insert(
            &provider,
            &json!({ "access": "a", "refresh": "r", "expires": now_ms(), "tokenUrl": token_url }),
        );

        f.credentials
            .token(
                &Client::new(),
                &remote(&same_origin_server, None),
                false,
                None,
                false,
            )
            .await
            .unwrap();

        assert_eq!(
            server.forms.lock().unwrap()[0],
            vec![
                ("grant_type".to_owned(), "refresh_token".to_owned()),
                ("refresh_token".to_owned(), "r".to_owned()),
            ]
        );
    }

    // spec: DMN-353
    #[tokio::test]
    async fn a_refresh_asks_again_for_the_scope_the_grant_holds_and_keeps_it() {
        let f = fixture();
        let (server, token_url) = token_server().await;
        let same_origin_server = token_url.replace("/token", "/mcp");
        let provider = format!("mcp_oauth:profile:default:{same_origin_server}");
        let id = f.insert(
            &provider,
            &json!({
                "access": "a",
                "refresh": "r",
                "expires": now_ms(),
                "tokenUrl": token_url,
                "scope": " api://client/.default offline_access ",
            }),
        );

        f.credentials
            .token(
                &Client::new(),
                &remote(&same_origin_server, None),
                false,
                None,
                false,
            )
            .await
            .unwrap();

        assert_eq!(
            server.forms.lock().unwrap()[0],
            vec![
                ("grant_type".to_owned(), "refresh_token".to_owned()),
                ("refresh_token".to_owned(), "r".to_owned()),
                (
                    "scope".to_owned(),
                    "api://client/.default offline_access".to_owned()
                ),
            ]
        );
        assert_eq!(
            f.data(id)["scope"],
            " api://client/.default offline_access "
        );
    }
}
