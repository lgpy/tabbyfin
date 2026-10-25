//! Remote control from other clients: the web dashboard or a phone app plays items here,
//! pauses, seeks, changes the volume or tracks, or shows a message. `live.rs` hands over the
//! commands from the session websocket; P3 brings the handlers.
//!
//! Transport controls go through `Services::control`, so in a SyncPlay group they become group
//! requests like the player's own buttons; playing items goes through the playback service,
//! which hands them to the group's queue there.

use jellyfin::models::BaseItemDto;

/// The fetched items in the requested order (the server returns them in its own), leaving
/// out ids it didn't return.
fn in_order(ids: &[String], items: Vec<BaseItemDto>) -> Vec<BaseItemDto> {
    let mut items: Vec<Option<BaseItemDto>> = items.into_iter().map(Some).collect();
    ids.iter()
        .filter_map(|id| {
            items
                .iter_mut()
                .find(|item| item.as_ref().is_some_and(|i| &i.id == id))?
                .take()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use jellyfin::models::BaseItemDto;

    use super::in_order;

    fn item(id: &str) -> BaseItemDto {
        serde_json::from_value(serde_json::json!({ "Id": id, "Name": id })).expect("item")
    }

    #[test]
    fn items_keep_the_requested_order() {
        let ids = ["c", "a", "b", "gone"].map(String::from);
        let items = in_order(&ids, vec![item("a"), item("b"), item("c")]);
        let order: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(order, ["c", "a", "b"]);
    }
}
