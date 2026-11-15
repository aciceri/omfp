//! JSON Schemas of the protocol types that travel outside method params and results: topic states, topic events and
//! errors (AGT-007). Together with [`method_schemas`](crate::method_schemas) they describe every frame a client
//! decodes, except thread events, which carry omp's own wire types (`omfp_core::ThreadEvent`).

use schemars::schema_for;
use serde_json::{Map, Value};

use crate::dto::{IndexSnapshot, JobState, RepoStatus, TerminalState, ViewsState};
use crate::envelope::{
    ErrorBody, ErrorCode, IndexEvent, JobEvent, TerminalEvent, TransientEvent, ViewsEvent,
    WorkspaceEvent,
};
use crate::topic::Topic;
use crate::windows::WindowRequest;

/// Schemas keyed by type name; each schema's `title` is that name.
#[must_use]
pub fn type_schemas() -> Value {
    let schemas = [
        ("Topic", schema_for!(Topic)),
        ("IndexSnapshot", schema_for!(IndexSnapshot)),
        ("IndexEvent", schema_for!(IndexEvent)),
        ("RepoStatus", schema_for!(RepoStatus)),
        ("WorkspaceEvent", schema_for!(WorkspaceEvent)),
        ("ViewsState", schema_for!(ViewsState)),
        ("ViewsEvent", schema_for!(ViewsEvent)),
        ("JobState", schema_for!(JobState)),
        ("JobEvent", schema_for!(JobEvent)),
        ("TerminalState", schema_for!(TerminalState)),
        ("TerminalEvent", schema_for!(TerminalEvent)),
        ("TransientEvent", schema_for!(TransientEvent)),
        ("WindowRequest", schema_for!(WindowRequest)),
        ("ErrorCode", schema_for!(ErrorCode)),
        ("ErrorBody", schema_for!(ErrorBody)),
    ];
    let mut map = Map::new();
    for (name, schema) in schemas {
        map.insert(name.to_owned(), schema.to_value());
    }
    Value::Object(map)
}
