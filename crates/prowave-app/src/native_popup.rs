//! Pop-up menus (a slot's menu in a channel strip, a right-click menu) as the operating
//! system's own menus where that is possible, as Logic's are: on macOS they are `NSMenu`s, with
//! the system's check marks, submenu arrows and keyboard handling. Elsewhere, and in tests, the
//! same menu is drawn by egui as before.
//!
//! A menu is described once, as a list of [`Item`]s that each carry the action to take, and
//! [`menu`] / [`context_menu`] return the action the user picked:
//!
//! ```ignore
//! if let Some(action) = native_popup::menu(&response, || items(...)) { apply(action) }
//! ```
//!
//! A native menu runs its own event loop until it closes. Opened from inside a frame it would
//! hold that frame, and the window would stand still under it (meters, playhead). So the menu
//! is opened by a run-loop timer just after the frame, where the window keeps drawing; the choice
//! comes back through the menu bar's event handler (`native_menu`) and is handed to the widget
//! that asked on a later frame. Measured on macOS: 60 frames a second with the menu open, none
//! when it is opened from inside the frame.
//!
//! Not done: Windows (the menus are egui's there; `muda` can show them, but opening one outside
//! the frame needs code that could not be tried), and the menus outside the channel strips.
use egui::{Response, Ui};

/// One line of a menu. `A` is what picking it means to the widget that shows the menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Item<A> {
    /// A line that can be picked: greyed out unless `enabled`, with a check mark when
    /// `checked`.
    Action {
        text: String,
        enabled: bool,
        checked: bool,
        action: A,
    },
    /// A heading: text that cannot be picked ("Recent").
    Label(String),
    Separator,
    Sub {
        text: String,
        enabled: bool,
        items: Vec<Item<A>>,
    },
}

impl<A> Item<A> {
    /// A plain line.
    pub fn action(text: impl Into<String>, action: A) -> Self {
        Item::Action {
            text: text.into(),
            enabled: true,
            checked: false,
            action,
        }
    }

    /// A line with a check mark when `checked`.
    pub fn checked(text: impl Into<String>, checked: bool, action: A) -> Self {
        Item::Action {
            text: text.into(),
            enabled: true,
            checked,
            action,
        }
    }

    /// A line that is there but cannot be picked (what Logic has and Pro Wave does not yet).
    pub fn disabled(text: impl Into<String>, action: A) -> Self {
        Item::Action {
            text: text.into(),
            enabled: false,
            checked: false,
            action,
        }
    }

    /// This line, greyed out unless `enabled`.
    pub fn enabled(mut self, on: bool) -> Self {
        match &mut self {
            Item::Action { enabled, .. } | Item::Sub { enabled, .. } => *enabled = on,
            Item::Label(_) | Item::Separator => {}
        }
        self
    }

    pub fn sub(text: impl Into<String>, items: Vec<Item<A>>) -> Self {
        Item::Sub {
            text: text.into(),
            enabled: true,
            items,
        }
    }
}

/// The menu drawn by egui; returns the action of the line that was clicked.
fn draw<A: Clone>(ui: &mut Ui, items: &[Item<A>]) -> Option<A> {
    let mut picked = None;
    for item in items {
        match item {
            Item::Action {
                text,
                enabled,
                checked,
                action,
            } => {
                let line = egui::Button::selectable(*checked, text.as_str());
                if ui.add_enabled(*enabled, line).clicked() {
                    picked = Some(action.clone());
                    ui.close();
                }
            }
            Item::Label(text) => {
                ui.weak(text.as_str());
            }
            Item::Separator => {
                ui.separator();
            }
            Item::Sub {
                text,
                enabled,
                items,
            } => {
                ui.add_enabled_ui(*enabled, |ui| {
                    ui.menu_button(text.as_str(), |ui| {
                        ui.set_min_width(190.0);
                        if let Some(action) = draw(ui, items) {
                            picked = Some(action);
                        }
                    });
                });
            }
        }
    }
    picked
}

/// Whether menus are the system's own here. `PROWAVE_EGUI_MENUS` (any value) keeps egui's.
fn native() -> bool {
    cfg!(all(target_os = "macos", not(test))) && std::env::var_os("PROWAVE_EGUI_MENUS").is_none()
}

/// The menu a click on `response` opens (a slot's menu). `items` is called when the menu is
/// needed. Returns the action picked, once, on the frame it is known.
pub fn menu<A: Clone + Send + 'static>(
    response: &Response,
    items: impl FnOnce() -> Vec<Item<A>>,
) -> Option<A> {
    show(response, response.clicked(), false, items)
}

/// The menu a right-click on `response` opens.
pub fn context_menu<A: Clone + Send + 'static>(
    response: &Response,
    items: impl FnOnce() -> Vec<Item<A>>,
) -> Option<A> {
    show(response, response.secondary_clicked(), true, items)
}

fn show<A: Clone + Send + 'static>(
    response: &Response,
    asked: bool,
    context: bool,
    items: impl FnOnce() -> Vec<Item<A>>,
) -> Option<A> {
    if !native() {
        let popup = if context {
            egui::Popup::context_menu(response)
        } else {
            egui::Popup::menu(response)
        };
        return popup
            .show(|ui| {
                ui.set_min_width(190.0);
                draw(ui, &items())
            })
            .and_then(|shown| shown.inner);
    }
    #[cfg(target_os = "macos")]
    {
        // The two menus of one widget are told apart, so a choice goes back to the right one.
        let owner = response.id.with(context);
        if asked {
            // Logic's menus open under the slot; a right-click menu opens at the pointer.
            let at = if context {
                response
                    .interact_pointer_pos()
                    .unwrap_or(response.rect.left_bottom())
            } else {
                response.rect.left_bottom()
            };
            mac::open(&response.ctx, owner, at, items());
        }
        mac::take(owner)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = asked;
        None
    }
}

/// A menu-bar event that belongs to a pop-up menu: true when `id` was one of its lines (the
/// caller, the menu bar's handler, then has nothing more to do with it).
pub fn route(id: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::route(id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        false
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::Item;
    use muda::{CheckMenuItem, ContextMenu, Menu, MenuItem, PredefinedMenuItem, Submenu};
    use std::any::Any;
    use std::cell::RefCell;
    use std::ffi::{c_char, c_void, CStr};
    use std::sync::Mutex;

    /// What every line's id starts with, so the menu bar's handler knows them.
    const PREFIX: &str = "prowave-popup:";

    /// The menu that is open or about to be, and the choice made in it.
    #[derive(Default)]
    struct State {
        owner: Option<egui::Id>,
        /// The action of each line, by the number in its id.
        actions: Vec<Box<dyn Any + Send>>,
        picked: Option<usize>,
        ctx: Option<egui::Context>,
    }

    static STATE: Mutex<Option<State>> = Mutex::new(None);

    thread_local! {
        /// The menu waiting to be shown, with where (in points of the window's view). It is
        /// made on the main thread, by the frame, and shown on it, by the run loop.
        static WAITING: RefCell<Option<(Menu, f64, f64)>> = const { RefCell::new(None) };
    }

    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *const c_void;
        fn objc_msgSend();
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFRunLoopCommonModes: *const c_void;
        fn CFRunLoopGetMain() -> *mut c_void;
        fn CFAbsoluteTimeGetCurrent() -> f64;
        fn CFRunLoopTimerCreate(
            allocator: *const c_void,
            fire: f64,
            interval: f64,
            flags: u64,
            order: isize,
            callout: extern "C" fn(*mut c_void, *mut c_void),
            context: *const c_void,
        ) -> *mut c_void;
        fn CFRunLoopAddTimer(run_loop: *mut c_void, timer: *mut c_void, mode: *const c_void);
        fn CFRelease(object: *const c_void);
    }

    /// Runs `work` once on the main thread's run loop, `after` seconds from now, in every mode
    /// (a menu being open is one). Unlike the main dispatch queue, which is serial, a timer
    /// does not hold up the others while its work is in a menu's loop.
    fn on_run_loop(after: f64, work: extern "C" fn(*mut c_void, *mut c_void)) {
        // SAFETY: Core Foundation calls with their own argument types; the run loop keeps the
        // timer after `CFRunLoopAddTimer`, so this reference to it is released.
        unsafe {
            let timer = CFRunLoopTimerCreate(
                std::ptr::null(),
                CFAbsoluteTimeGetCurrent() + after,
                0.0,
                0,
                0,
                work,
                std::ptr::null(),
            );
            if !timer.is_null() {
                CFRunLoopAddTimer(CFRunLoopGetMain(), timer, kCFRunLoopCommonModes);
                CFRelease(timer);
            }
        }
    }

    /// Sends a message that takes nothing and returns an object.
    ///
    /// # Safety
    /// `object` is a live Objective-C object (or class) that answers `selector` that way.
    unsafe fn send(object: *mut c_void, selector: &CStr) -> *mut c_void {
        // SAFETY: the caller's contract; `objc_msgSend` is called through the method's type.
        unsafe {
            let function: unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as *const c_void);
            function(object, sel_registerName(selector.as_ptr()))
        }
    }

    /// Adds `items` to a menu or submenu through `append`, numbering the actions as they come.
    fn fill<A: Send + 'static>(
        append: &dyn Fn(&dyn muda::IsMenuItem),
        items: Vec<Item<A>>,
        actions: &mut Vec<Box<dyn Any + Send>>,
    ) {
        for item in items {
            match item {
                Item::Action {
                    text,
                    enabled,
                    checked,
                    action,
                } => {
                    let id = format!("{PREFIX}{}", actions.len());
                    actions.push(Box::new(action));
                    if checked {
                        append(&CheckMenuItem::with_id(id, text, enabled, true, None));
                    } else {
                        append(&MenuItem::with_id(id, text, enabled, None));
                    }
                }
                Item::Label(text) => append(&MenuItem::new(text, false, None)),
                Item::Separator => append(&PredefinedMenuItem::separator()),
                Item::Sub {
                    text,
                    enabled,
                    items,
                } => {
                    let sub = Submenu::new(text, enabled);
                    fill(
                        &|line| {
                            let _ = sub.append(line);
                        },
                        items,
                        actions,
                    );
                    append(&sub);
                }
            }
        }
    }

    /// Makes the menu and asks the main queue to show it once this frame is over. A menu that
    /// is already waiting or open stays the one.
    pub fn open<A: Send + 'static>(
        ctx: &egui::Context,
        owner: egui::Id,
        at: egui::Pos2,
        items: Vec<Item<A>>,
    ) {
        let Ok(mut state) = STATE.lock() else {
            return;
        };
        if state.as_ref().is_some_and(|s| s.picked.is_none()) {
            return;
        }
        let menu = Menu::new();
        let mut actions = Vec::new();
        fill(
            &|line| {
                let _ = menu.append(line);
            },
            items,
            &mut actions,
        );
        *state = Some(State {
            owner: Some(owner),
            actions,
            picked: None,
            ctx: Some(ctx.clone()),
        });
        // egui's points are the view's points times the interface zoom.
        let zoom = f64::from(ctx.zoom_factor());
        WAITING.with(|w| {
            *w.borrow_mut() = Some((menu, f64::from(at.x) * zoom, f64::from(at.y) * zoom))
        });
        on_run_loop(0.0, show_waiting);
    }

    /// Run by the run loop, outside any frame: shows the waiting menu and stays in the
    /// system's menu loop until it closes. The window keeps drawing meanwhile.
    extern "C" fn show_waiting(_: *mut c_void, _: *mut c_void) {
        let Some((menu, x, y)) = WAITING.with(|w| w.borrow_mut().take()) else {
            return;
        };
        // SAFETY: AppKit calls on the main thread (the run loop's); the view is the
        // content view of the app's window, alive while the menu is.
        let picked = unsafe {
            let app = send(
                objc_getClass(c"NSApplication".as_ptr()),
                c"sharedApplication",
            );
            let mut window = send(app, c"keyWindow");
            if window.is_null() {
                window = send(app, c"mainWindow");
            }
            let view = if window.is_null() {
                std::ptr::null_mut()
            } else {
                send(window, c"contentView")
            };
            !view.is_null()
                && menu.show_context_menu_for_nsview(
                    view,
                    Some(muda::dpi::Position::Logical(
                        muda::dpi::LogicalPosition::new(x, y),
                    )),
                )
        };
        // Closed without a choice: nobody is waiting for one any more. With a choice, the menu
        // bar's handler has it (or will: it runs on this thread too).
        if let Ok(mut state) = STATE.lock() {
            if !picked {
                *state = None;
            } else if let Some(ctx) = state.as_ref().and_then(|s| s.ctx.clone()) {
                ctx.request_repaint();
            }
        }
    }

    pub fn route(id: &str) -> bool {
        let Some(number) = id
            .strip_prefix(PREFIX)
            .and_then(|n| n.parse::<usize>().ok())
        else {
            return false;
        };
        if let Ok(mut state) = STATE.lock() {
            if let Some(state) = state.as_mut() {
                state.picked = Some(number);
            }
        }
        true
    }

    /// The choice made in `owner`'s menu, if one was made since the last call.
    pub fn take<A: Clone + 'static>(owner: egui::Id) -> Option<A> {
        let mut state = STATE.lock().ok()?;
        let open = state.as_ref()?;
        if open.owner != Some(owner) {
            return None;
        }
        let number = open.picked?;
        let action = open.actions.get(number)?.downcast_ref::<A>().cloned();
        *state = None;
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builders_say_what_a_line_is() {
        assert_eq!(
            Item::action("Open", 1),
            Item::Action {
                text: "Open".into(),
                enabled: true,
                checked: false,
                action: 1
            }
        );
        assert!(matches!(
            Item::checked("Pre Fader", true, 2),
            Item::Action { checked: true, .. }
        ));
        assert!(matches!(
            Item::disabled("Post Pan", 3),
            Item::Action { enabled: false, .. }
        ));
        assert!(matches!(
            Item::action("No Send", 4).enabled(false),
            Item::Action { enabled: false, .. }
        ));
        assert!(matches!(
            Item::sub("Bus", vec![Item::action("Bus 1", 5)]).enabled(false),
            Item::Sub { enabled: false, .. }
        ));
    }

    #[test]
    fn a_click_on_a_line_of_the_drawn_menu_gives_its_action() {
        use crate::ui_test::Harness;
        let mut h = Harness::new(300.0, 300.0);
        let items = vec![
            Item::Label("Recent".into()),
            Item::action("First", 'a'),
            Item::Separator,
            Item::disabled("Greyed", 'b'),
            Item::checked("Last", true, 'c'),
        ];
        let mut picked = None;
        let frame = |ui: &mut Ui, picked: &mut Option<char>| {
            if let Some(action) = draw(ui, &items) {
                *picked = Some(action);
            }
        };
        h.settle(|ui| frame(ui, &mut picked));
        // The lines are stacked from the top; try the rows the greyed and the last can be in.
        for y in (30..120).step_by(4) {
            h.click(egui::pos2(30.0, y as f32), |ui| frame(ui, &mut picked));
            assert_ne!(picked, Some('b'), "a greyed line cannot be picked");
            if picked == Some('c') {
                break;
            }
            picked = None;
        }
        assert_eq!(picked, Some('c'));
        assert!(
            !route("something else"),
            "only a pop-up's own ids are its lines"
        );
    }
}
