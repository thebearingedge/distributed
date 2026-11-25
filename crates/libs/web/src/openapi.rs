//! `OpenAPI` helpers: the `/openapi.json` handler and the standard
//! error-response transforms that every operation should apply
//! (`standard_get_errors`/`standard_post_errors`/`standard_put_errors`/
//! `standard_delete_errors`).

use std::{future::Ready, sync::Arc};

use aide::{
    OperationOutput,
    openapi::{Header, HeaderStyle, OpenApi, ParameterSchemaOrContent, ReferenceOr, SchemaObject},
    transform::TransformOperation,
};
use axum::{Extension, Json};
use schemars::json_schema;

use crate::api_error::{ErrorBody, ValidationErrorBody};

/// Serves the generated `OpenAPI` document as JSON.
///
/// Mount it on the `/openapi.json` route; it requires the
/// `Extension<Arc<OpenApi>>` layer installed by `ApiRouter::finish_api`.
pub fn openapi_json(Extension(api): Extension<Arc<OpenApi>>) -> Ready<Json<Arc<OpenApi>>> {
    std::future::ready(Json(api))
}

/// The standard error responses every operation can produce, regardless of
/// method: 400 (bad request), 422 (deserialization/schema violation), 500
/// (internal), and 503 (unavailable, including request timeouts).
fn standard_errors(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.response::<400, Json<ErrorBody>>()
        .response::<422, Json<ValidationErrorBody>>()
        .response::<500, Json<ErrorBody>>()
        .response::<503, Json<ErrorBody>>()
}

/// Documents the standard error responses of a GET operation (400/422/500/503).
pub fn standard_get_errors(op: TransformOperation<'_>) -> TransformOperation<'_> {
    standard_errors(op)
}

/// Documents the standard error responses of a POST operation: the GET set
/// plus 413 (payload too large) and 415 (unsupported media type).
pub fn standard_post_errors(op: TransformOperation<'_>) -> TransformOperation<'_> {
    standard_errors(op)
        .response::<413, Json<ErrorBody>>()
        .response::<415, Json<ErrorBody>>()
}

/// Documents the standard error responses of a PUT operation: the GET set
/// plus 413 (payload too large) and 415 (unsupported media type).
pub fn standard_put_errors(op: TransformOperation<'_>) -> TransformOperation<'_> {
    standard_errors(op)
        .response::<413, Json<ErrorBody>>()
        .response::<415, Json<ErrorBody>>()
}

/// Documents the standard error responses of a DELETE operation
/// (400/422/500/503). DELETE carries no request body, so unlike post/put
/// there is no 415 response.
pub fn standard_delete_errors(op: TransformOperation<'_>) -> TransformOperation<'_> {
    standard_errors(op)
}

/// Documents a string-valued response header named `name` on the `CODE`
/// response of the operation, alongside the response that `T` documents.
///
/// Use it in place of `TransformOperation::response::<CODE, T>` when a
/// successful create response also carries a `Location` header (or any other
/// string header).
pub fn response_header<'a, T, const CODE: u16>(
    op: TransformOperation<'a>,
    name: &'static str,
    description: &str,
) -> TransformOperation<'a>
where
    T: OperationOutput,
{
    op.response_with::<CODE, T, _>(|mut response| {
        response.inner().headers.insert(
            name.to_owned(),
            ReferenceOr::Item(Header {
                description: Some(description.to_owned()),
                style: HeaderStyle::Simple,
                required: true,
                deprecated: None,
                format: ParameterSchemaOrContent::Schema(SchemaObject {
                    json_schema: json_schema!({ "type": "string" }),
                    external_docs: None,
                    example: None,
                }),
                example: None,
                examples: [].into(),
                extensions: [].into(),
            }),
        );
        response
    })
}

#[cfg(test)]
mod tests {
    use aide::{
        openapi::{Operation, ReferenceOr, StatusCode},
        transform::TransformOperation,
    };
    use axum::Json;
    use serde_json::json;
    use serde_json_assert::assert_json_include;

    use super::{
        response_header, standard_delete_errors, standard_get_errors, standard_post_errors,
        standard_put_errors,
    };

    fn documented_codes(
        transform: fn(TransformOperation<'_>) -> TransformOperation<'_>,
    ) -> Vec<StatusCode> {
        let mut operation = Operation::default();
        drop(transform(TransformOperation::new(&mut operation)));
        operation
            .responses
            .expect("responses")
            .responses
            .into_keys()
            .collect()
    }

    #[test]
    fn post_and_put_document_413() {
        for codes in [
            documented_codes(standard_post_errors),
            documented_codes(standard_put_errors),
        ] {
            assert!(codes.contains(&StatusCode::Code(413)));
            assert!(codes.contains(&StatusCode::Code(415)));
        }
    }

    #[test]
    fn get_and_delete_do_not_document_413() {
        for codes in [
            documented_codes(standard_get_errors),
            documented_codes(standard_delete_errors),
        ] {
            assert!(!codes.contains(&StatusCode::Code(413)));
        }
    }

    #[test]
    fn response_header_documents_a_required_string_header() {
        let mut operation = Operation::default();
        let op = response_header::<Json<String>, 201>(
            TransformOperation::new(&mut operation),
            "Location",
            "The path of the new todo, relative to the service root.",
        );
        drop(op);

        let responses = operation
            .responses
            .as_ref()
            .expect("the operation should document its responses");
        let ReferenceOr::Item(created) = responses
            .responses
            .get(&StatusCode::Code(201))
            .expect("the operation should document a 201 response")
        else {
            panic!("the 201 response should be documented inline");
        };
        let ReferenceOr::Item(location) = created
            .headers
            .get("Location")
            .expect("the 201 response should document the Location header")
        else {
            panic!("the Location header should be documented inline");
        };

        let header = serde_json::to_value(location).expect("the header should serialize");
        assert_json_include!(actual: header, expected: json!({
            "required": true,
            "description": "The path of the new todo, relative to the service root.",
            "schema": { "type": "string" }
        }));
    }
}
