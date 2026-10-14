use serde::Serialize;

use crate::client::Query;
use crate::models::{AuthenticationResult, QuickConnectResult, UserDto};
use crate::{Client, Result};

impl Client {
    /// Logs in with a username and password. Use the returned token with [`Client::with_token`].
    pub async fn authenticate_by_name(&self, username: &str, password: &str) -> Result<AuthenticationResult> {
        #[derive(Serialize)]
        #[serde(rename_all = "PascalCase")]
        struct Body<'a> {
            username: &'a str,
            pw: &'a str,
        }
        let body = Body {
            username,
            pw: password,
        };
        self.post("/Users/AuthenticateByName", &Query::new(), Some(&body)).await
    }

    pub async fn quick_connect_enabled(&self) -> Result<bool> {
        self.get("/QuickConnect/Enabled", &Query::new()).await
    }

    /// Starts a Quick Connect request. Show `code` to the user and poll
    /// [`Client::quick_connect_state`] with `secret` until it is authenticated.
    ///
    /// This is a `POST`: the `GET` form was removed in Jellyfin 12.0.
    pub async fn quick_connect_initiate(&self) -> Result<QuickConnectResult> {
        self.post::<(), _>("/QuickConnect/Initiate", &Query::new(), None).await
    }

    pub async fn quick_connect_state(&self, secret: &str) -> Result<QuickConnectResult> {
        self.get("/QuickConnect/Connect", Query::new().push("secret", secret)).await
    }

    /// Exchanges an approved Quick Connect secret for a session.
    pub async fn authenticate_with_quick_connect(&self, secret: &str) -> Result<AuthenticationResult> {
        #[derive(Serialize)]
        #[serde(rename_all = "PascalCase")]
        struct Body<'a> {
            secret: &'a str,
        }
        self.post("/Users/AuthenticateWithQuickConnect", &Query::new(), Some(&Body { secret }))
            .await
    }

    pub async fn current_user(&self) -> Result<UserDto> {
        self.get("/Users/Me", &Query::new()).await
    }

    /// Ends the session and revokes this client's token.
    pub async fn logout(&self) -> Result<()> {
        self.post_no_content::<()>("/Sessions/Logout", &Query::new(), None).await
    }
}
