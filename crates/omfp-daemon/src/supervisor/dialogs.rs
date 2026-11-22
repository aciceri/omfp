//! Answers to omp's interactive requests (DMN-050..055): `select`, `confirm`, `input`, `editor`, `ask` and `form`
//! dialogs.

use omfp_core::wire::LitTrue;
use omfp_core::{Resolution, ThreadEvent, validate_ask_answers, validate_form_data};
use omfp_protocol::InteractionResponse;
use omp_rpc::{
    AnswersUiResponse, CancelUiResponse, ConfirmUiResponse, DataUiResponse, ExtensionUiRequest,
    ExtensionUiResponse, ValueUiResponse,
};

use super::{Result, Supervisor, SupervisorError, lock};

impl Supervisor {
    /// Answers a pending interactive request: the first valid answer is forwarded and recorded, later ones get
    /// `already_resolved` (DMN-051); `ask` and `form` answers are validated first (DMN-054, DMN-055).
    pub async fn respond(
        &self,
        id: &str,
        interaction_id: &str,
        response: InteractionResponse,
        client: &str,
    ) -> Result<()> {
        let actor = self.actor(id).await?;
        let _answers = lock(&actor.answers);
        let request = actor.log.with_state(|state| {
            state
                .interactions
                .iter()
                .find(|request| dialog_id(request) == Some(interaction_id))
                .cloned()
        });
        let (Some(request), Some(process)) = (request, actor.process()) else {
            return Err(SupervisorError::AlreadyResolved(interaction_id.to_owned()));
        };
        let reply = ui_response(&request, interaction_id, response)
            .map_err(SupervisorError::InvalidResponse)?;
        // `respond` only queues the frame, so holding the answer lock across it is cheap.
        process.respond(reply)?;
        self.push(
            &actor,
            ThreadEvent::InteractionResolved {
                id: interaction_id.to_owned(),
                resolution: Resolution::Client {
                    client: client.to_owned(),
                },
            },
        );
        Ok(())
    }
}

/// The id of a dialog request; `None` for presentation-only methods.
pub(super) fn dialog_id(request: &ExtensionUiRequest) -> Option<&str> {
    match request {
        ExtensionUiRequest::Select(select) => Some(&select.id),
        ExtensionUiRequest::Confirm(confirm) => Some(&confirm.id),
        ExtensionUiRequest::Input(input) => Some(&input.id),
        ExtensionUiRequest::Editor(editor) => Some(&editor.id),
        ExtensionUiRequest::Ask(ask) => Some(&ask.id),
        ExtensionUiRequest::Form(form) => Some(&form.id),
        _ => None,
    }
}

/// The omp answer for `request`, checked against its kind: `select` takes one of its options, `confirm` a boolean,
/// `ask` answers in omp's shape (DMN-054), `form` its submitted data (DMN-055); any request can be cancelled.
fn ui_response(
    request: &ExtensionUiRequest,
    id: &str,
    response: InteractionResponse,
) -> std::result::Result<ExtensionUiResponse, String> {
    let id = id.to_owned();
    match (request, response) {
        (_, InteractionResponse::Cancelled) => {
            Ok(ExtensionUiResponse::CancelUiResponse(CancelUiResponse {
                id,
                cancelled: LitTrue,
                timed_out: None,
            }))
        }
        (ExtensionUiRequest::Select(select), InteractionResponse::Value(value)) => {
            if select.options.contains(&value) {
                Ok(ExtensionUiResponse::ValueUiResponse(ValueUiResponse {
                    id,
                    value,
                }))
            } else {
                Err(format!(
                    "{value:?} is not one of the options {:?}",
                    select.options
                ))
            }
        }
        (
            ExtensionUiRequest::Input(_) | ExtensionUiRequest::Editor(_),
            InteractionResponse::Value(value),
        ) => Ok(ExtensionUiResponse::ValueUiResponse(ValueUiResponse {
            id,
            value,
        })),
        (ExtensionUiRequest::Confirm(_), InteractionResponse::Confirmed(confirmed)) => {
            Ok(ExtensionUiResponse::ConfirmUiResponse(ConfirmUiResponse {
                id,
                confirmed,
            }))
        }
        (ExtensionUiRequest::Ask(ask), InteractionResponse::Answers(answers)) => {
            validate_ask_answers(ask, &answers)?;
            Ok(ExtensionUiResponse::AnswersUiResponse(AnswersUiResponse {
                id,
                answers,
            }))
        }
        (ExtensionUiRequest::Form(_), InteractionResponse::Data(data)) => {
            validate_form_data(&data)?;
            Ok(ExtensionUiResponse::DataUiResponse(DataUiResponse {
                id,
                data,
            }))
        }
        (_, response) => Err(format!(
            "{} cannot answer this request",
            match response {
                InteractionResponse::Value(_) => "a value",
                InteractionResponse::Confirmed(_) => "a confirmation",
                InteractionResponse::Answers(_) => "ask answers",
                InteractionResponse::Cancelled => "a cancellation",
                InteractionResponse::Data(_) => "form data",
            }
        )),
    }
}
