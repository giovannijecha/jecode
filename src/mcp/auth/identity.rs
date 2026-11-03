/// Public native-client metadata used when an authorization server supports CIMD.
const CLIENT_ID: &str = "https://giovannijecha.github.io/jecode/mcp-oauth-client.json";

pub(super) fn default_client_id() -> &'static str {
    CLIENT_ID
}

#[cfg(test)]
mod tests {
    use super::{super::discovery, default_client_id};
    use crate::json::{self, Value};

    #[test]
    fn bundled_identity_matches_public_document_and_native_callback() {
        let metadata = json::parse(include_str!("../../../.well-known/mcp-oauth-client.json"))
            .expect("bundled CIMD must be JSON");
        assert_eq!(
            metadata.get("client_id").and_then(Value::as_str),
            Some(default_client_id())
        );
        assert_eq!(
            metadata.get("client_name").and_then(Value::as_str),
            Some("Jecode")
        );
        assert_eq!(
            metadata
                .get("token_endpoint_auth_method")
                .and_then(Value::as_str),
            Some("none")
        );
        assert!(metadata.get("client_secret").is_none());
        let redirects = metadata
            .get("redirect_uris")
            .and_then(Value::as_array)
            .expect("native client needs redirect URIs");
        assert!(
            redirects
                .iter()
                .filter_map(Value::as_str)
                .any(|uri| { discovery::callback_matches(uri, "http://127.0.0.1:49281/callback") })
        );
    }
}
