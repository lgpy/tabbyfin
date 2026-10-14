//! App-wide notification that the user's watch data changed (played state, resume points),
//! so pages showing it can refresh.

use gpui_kit::*;

pub struct UserDataChanged;

pub struct Changes;

impl EventEmitter<UserDataChanged> for Changes {}

struct GlobalChanges(Entity<Changes>);

impl Global for GlobalChanges {}

impl Changes {
    pub fn init(cx: &mut App) {
        let changes = cx.new(|_| Changes);
        cx.set_global(GlobalChanges(changes));
    }

    pub fn global(cx: &App) -> Entity<Changes> {
        cx.global::<GlobalChanges>().0.clone()
    }

    pub fn notify_user_data(cx: &mut App) {
        Self::global(cx).update(cx, |_, cx| cx.emit(UserDataChanged));
    }
}
