//! Shelves that scroll sideways only by dragging (or their arrow buttons): the mouse wheel
//! keeps scrolling the page, and a press on a shelf followed by a move drags it. A drag
//! doesn't open the card it ends on.
//!
//! A page keeps one [`RailDrag`], decorates each shelf's row with [`RailDrag::rail`], tracks
//! the mouse over the whole page with [`RailDrag::track`], and wraps its cards' click handlers
//! in [`RailDrag::guard`].

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// How far the mouse moves with the button down before a press on a shelf becomes a drag.
const DRAG_THRESHOLD: Pixels = px(4.);

#[derive(Clone, Default)]
pub struct RailDrag(Rc<RefCell<State>>);

#[derive(Default)]
struct State {
    /// The shelf being pressed or dragged, while the button is down.
    active: Option<Active>,
    /// Whether the last press on a shelf became a drag, so its release doesn't open a card.
    dragged: bool,
}

/// A press on a shelf: its scroll, and where the mouse and the scroll were then.
struct Active {
    scroll: ScrollHandle,
    start_x: Pixels,
    start_offset: Pixels,
}

impl RailDrag {
    /// Decorates a shelf's row, which tracks `scroll`: the wheel doesn't move it, and a press
    /// on it can start a drag.
    pub fn rail(&self, scroll: &ScrollHandle) -> impl FnOnce(Stateful<Div>) -> Stateful<Div> + 'static {
        let this = self.clone();
        let scroll = scroll.clone();
        move |rail| {
            let wheel_scroll = scroll.clone();
            rail.on_scroll_wheel(move |event, window, _| undo_wheel_scroll(&wheel_scroll, event, window))
                .on_mouse_down(MouseButton::Left, move |event, _, _| {
                    let mut state = this.0.borrow_mut();
                    state.dragged = false;
                    state.active = Some(Active {
                        start_x: event.position.x,
                        start_offset: scroll.offset().x,
                        scroll: scroll.clone(),
                    });
                })
        }
    }

    /// Follows the mouse over the page while a shelf is pressed, so a drag continues off the
    /// shelf it started on, and ends it on release anywhere.
    pub fn track<E: InteractiveElement + Styled + FluentBuilder>(&self, element: E) -> E {
        let (moved, released, released_out) = (self.clone(), self.clone(), self.clone());
        element
            .when(self.is_dragging(), |el| el.cursor(CursorStyle::ClosedHand))
            .on_mouse_move(move |event, window, _| moved.drag(event, window))
            .on_mouse_up(MouseButton::Left, move |_, window, _| released.end(window))
            .on_mouse_up_out(MouseButton::Left, move |_, window, _| released_out.end(window))
    }

    /// A card's click handler that does nothing when the press became a drag.
    pub fn guard<E: 'static>(
        &self,
        on_click: impl Fn(&E, &mut Window, &mut App) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let this = self.clone();
        move |event, window, cx| {
            if !this.0.borrow().dragged {
                on_click(event, window, cx);
            }
        }
    }

    fn is_dragging(&self) -> bool {
        let state = self.0.borrow();
        state.dragged && state.active.is_some()
    }

    /// Scrolls the pressed shelf with the mouse once it has moved far enough to be a drag.
    fn drag(&self, event: &MouseMoveEvent, window: &mut Window) {
        let mut state = self.0.borrow_mut();
        let Some(active) = &state.active else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            state.active = None;
            window.refresh();
            return;
        }
        let dx = event.position.x - active.start_x;
        if !state.dragged && dx.abs() < DRAG_THRESHOLD {
            return;
        }
        let mut offset = active.scroll.offset();
        offset.x = (active.start_offset + dx).clamp(-active.scroll.max_offset().x, px(0.));
        active.scroll.set_offset(offset);
        state.dragged = true;
        window.refresh();
    }

    fn end(&self, window: &mut Window) {
        if self.0.borrow_mut().active.take().is_some() {
            window.refresh();
        }
    }
}

/// Takes back the sideways scroll GPUI's own wheel handler just gave `scroll`, so the wheel
/// keeps scrolling the page. GPUI runs the element's own handler first (bubble listeners run
/// last-registered first, and an element's scroll handler is registered after its
/// `on_scroll_wheel` listeners), and it turns a vertical wheel into a sideways scroll on a row
/// that only scrolls sideways.
fn undo_wheel_scroll(scroll: &ScrollHandle, event: &ScrollWheelEvent, window: &Window) {
    let delta = event.delta.pixel_delta(window.line_height());
    let applied = if delta.x.is_zero() { delta.y } else { delta.x };
    let mut offset = scroll.offset();
    offset.x -= applied;
    scroll.set_offset(offset);
}
