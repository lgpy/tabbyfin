use serde::Serialize;

use crate::client::Query;
use crate::models::{
    AuthenticationResult, Culture, DisplayPreferences, QuickConnectResult, UserConfigurationChanges, UserDto,
};
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
        let body = Body { username, pw: password };
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
        self.get("/QuickConnect/Connect", Query::new().push("secret", secret))
            .await
    }

    /// Exchanges an approved Quick Connect secret for a session.
    pub async fn authenticate_with_quick_connect(&self, secret: &str) -> Result<AuthenticationResult> {
        #[derive(Serialize)]
        #[serde(rename_all = "PascalCase")]
        struct Body<'a> {
            secret: &'a str,
        }
        self.post(
            "/Users/AuthenticateWithQuickConnect",
            &Query::new(),
            Some(&Body { secret }),
        )
        .await
    }

    pub async fn current_user(&self) -> Result<UserDto> {
        self.get("/Users/Me", &Query::new()).await
    }

    /// Saves `changes` to the signed-in user's configuration. The server replaces the whole
    /// configuration with what it's sent, so it's read first and sent back with only the
    /// changed fields different: the ones [`UserConfiguration`] doesn't model (the cast
    /// receiver, the ordered views, …) are kept as they are. `POST /Users/Configuration` is
    /// there since 10.9 and acts on the caller without a `userId`.
    ///
    /// [`UserConfiguration`]: crate::models::UserConfiguration
    pub async fn update_user_configuration(&self, changes: &UserConfigurationChanges) -> Result<()> {
        let mut user: serde_json::Value = self.get("/Users/Me", &Query::new()).await?;
        let mut config = match user.get_mut("Configuration").map(serde_json::Value::take) {
            Some(serde_json::Value::Object(config)) => config,
            _ => serde_json::Map::new(),
        };
        let changed = serde_json::to_value(changes).expect("strings and flags serialize");
        if let serde_json::Value::Object(changed) = changed {
            config.extend(changed);
        }
        self.post_no_content("/Users/Configuration", &Query::new(), Some(&config))
            .await
    }

    /// The languages the server knows, for a language preference's choices.
    pub async fn cultures(&self) -> Result<Vec<Culture>> {
        self.get("/Localization/Cultures", &Query::new()).await
    }

    /// Display preferences another client stored on the server, e.g. `("usersettings",
    /// "emby")` for the web client's Settings → Home.
    pub async fn display_preferences(&self, id: &str, client: &str) -> Result<DisplayPreferences> {
        let mut q = Query::new();
        q.push("client", client);
        self.get(&format!("/DisplayPreferences/{id}"), &q).await
    }

    /// Ends the session and revokes this client's token.
    pub async fn logout(&self) -> Result<()> {
        self.post_no_content::<()>("/Sessions/Logout", &Query::new(), None)
            .await
    }
}
