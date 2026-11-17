//! The sign-in family and two small UI types: the server operations (`HSM_SERV_OP`), the sign-in form's controls
//! (`SIGNIN_CNTRL`, `SI_EDT_EMPT`, `HSM_KEYB_ENT`, `SI_FIRST_PRT`, `SI_ADD_CNTRL`, `SI_SHOW_ERR`), the session
//! variable writer (`WRITESESSION`) and the string-set loader (`LOADSTRSET`, the credits). Engine effects are
//! [`Request`]s, engine state [`Query`]s (both name the original's code). Online-only branches are ported as read
//! and answered offline by the defaults of the queries.

use mw_formats::hsm::key;

use super::{FALSE, Query, TRUE, ask, ask_bool, param_int, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("HSM_SERV_OP", |_| ServerOperation::default());
    r.register_fn("SIGNIN_CNTRL", |_| SignInControl);
    r.register_fn("SI_EDT_EMPT", |_| EditTextEmpty);
    r.register_fn("HSM_KEYB_ENT", |_| KeyboardEntry);
    r.register_fn("SI_FIRST_PRT", |_| FirstPartySignIn::default());
    r.register_fn("SI_ADD_CNTRL", |_| AddControls::default());
    r.register_fn("SI_SHOW_ERR", |_| ShowSignInError::default());
    r.register_fn("WRITESESSION", |_| WriteSession);
    r.register_fn("LOADSTRSET", |_| LoadStringSet::default());
}

/// The operations of the table at 0xfd5208 (20 B entries: name, name string, start function, error handler,
/// success handler), in table order; the parameter `OPERATION` is compared with them case-sensitively
/// (`FUN_00745a90`).
pub const SERVER_OPERATIONS: [&str; 26] = [
    "Connect",
    "Login",
    "PCLogin",
    "ConsoleSilentLogin",
    "SilentLogin",
    "PrepareCreateAccount",
    "SetAccountInfo",
    "CreateAccount",
    "AcceptTofS",
    "DeclineTofS",
    "CancelLogin",
    "AddPersona",
    "ForgotPassword",
    "UpdateEMail",
    "UpdatePassword",
    "UpdateSpammable",
    "GetAccountInfo",
    "ReactivateAccount",
    "EntitleGame",
    "RetrieveToS",
    "RetrievePrivacy",
    "UnlockKey",
    "GetEntitlements",
    "HasOriginID",
    "TimeStampPostLogin",
    "OnlinePassKillSwitch",
];

/// The state of "no operation running" (+0x88 = 0x1a).
const OP_NONE: u32 = 26;
/// Which error handler the table's entries use: 0 = 0x73f4d0 (the generic one), 1 = 0x73f620 (`Login`,
/// `PCLogin`), 2 = a handler this port does not read (0x73f540 / 0x73f590 / 0x73f840 / 0x73fa90 / 0x73fc60 /
/// 0x73fc80 / 0x73fcc0 / 0x73fcf0 / 0x73fd30 / 0x73fdf0: online account flows, answered like the generic one).
fn error_handler(op: u32) -> u8 {
    match op {
        0 | 9 | 11 | 13 | 14 | 15 | 16 | 17 | 21 | 23 | 25 => 0,
        1 | 2 => 1,
        _ => 2,
    }
}

/// "Server Operation" (`HSM_SERV_OP`, factory 0x77ca70, vtable 0xfd9edc, 0xc8 B): one Blaze / Origin request
/// of the table above. Enter (0x750120) finds the operation (`FUN_00745a90`; unknown = state 26, the start
/// not run), runs its start function (+8, [`Request::ServerOpStart`]; true = request sent) and defaults the exit
/// to `SUCCESS` (+0x48 = 0x117aba8). The update (0x73f3c0): start failed → `FAILURE`; the request object
/// pumped (`FUN_004c4b30`); an error (+0x5c, [`Query::ServerOpError`]) → the operation's error handler (+0xc);
/// else when done (+0x58 = 2, [`Query::ServerOpState`]) the success handler of the *current* state (+0x10 of
/// that entry; state 0 `Connect` (0x7400b0) ends and starts the requested operation, the others are the engine's:
/// [`Request::ServerOpHandle`], the next state from [`Query::ServerOpNextState`]); the exit is the default once
/// the state is 26. The login operations (0x750210 / 0x7502d0) run `Connect` first unless the network layer is
/// up ([`Query::ServerOpNetReady`]). Error handlers: the generic 0x73f4d0 (error 0x11 exits `NO_PERSONA`,
/// else reports the code, `FAILURE`) and 0x73f620 of the logins (0x10 `CREATE`, 0x11 `NO_PERSONA`, 0xe
/// `PC_LOGIN`, 0x13 `SHOW_TOS`, 0x15 `WRONG_PWD`, 0x16 `ENTITLE_GAM`, else report + `FAILURE`); the other
/// handlers are answered like the generic one. Exit begin (0x73f450) cancels a request that is neither idle nor
/// done; exit update (0x73f490) acknowledges a done one (`FUN_004c2870`) and finishes when idle.
/// Offline (the query defaults: done with a generic error) every operation ends `FAILURE`, `Connect` first.
#[derive(Default)]
pub struct ServerOperation {
    /// +0x88: the current state (an operation index, 26 = none).
    state: u32,
    /// +0x8c: the requested operation.
    requested: u32,
    /// +0x90: the start function succeeded.
    started: bool,
    /// The request was acknowledged by the exit update: the object is idle again.
    acked: bool,
}

impl ServerOperation {
    /// The start function of operation `op` (+8 of its entry).
    fn start(&mut self, cx: &mut Cx, op: u32) -> bool {
        if matches!(op, 1..=3) && !ask_bool(cx, Query::ServerOpNetReady) {
            // 0x750210 / 0x7502d0: no connection yet, `Connect` (0x7501a0) is sent instead.
            self.state = 0;
            cx.host.request(Request::ServerOpStart { op: 0 });
            return true;
        }
        self.state = op;
        cx.host.request(Request::ServerOpStart { op });
        ask_bool(cx, Query::ServerOpStarted { op })
    }

    fn error(&mut self, cx: &mut Cx, code: u32) {
        let exit = match (error_handler(self.requested), code) {
            (_, 0x11) => key("NO_PERSONA"),
            (1, 0x10) => key("CREATE"),
            (1, 0xe) => key("PC_LOGIN"),
            (1, 0x13) => {
                cx.host.request(Request::ServerOpTermsRequired);
                key("SHOW_TOS")
            }
            (1, 0x15) => key("WRONG_PWD"),
            (1, 0x16) => key("ENTITLE_GAM"),
            _ => {
                // The error is shown (`FUN_008820c0`), the exit is the failure.
                cx.host.request(Request::ServerOpReport { code });
                cx.set_exit(key("FAILURE"));
                return;
            }
        };
        // These end the busy indicator only (`FUN_00882d00`).
        cx.host.request(Request::ServerOpReport { code: 0 });
        cx.set_exit(exit);
    }
}

impl Behaviour for ServerOperation {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.state = OP_NONE;
        self.requested = OP_NONE;
        self.started = false;
        self.acked = false;
        let name = cx.param_text("OPERATION");
        if let Some(op) = SERVER_OPERATIONS.iter().position(|n| *n == name) {
            self.requested = op as u32;
            self.started = self.start(cx, op as u32);
        }
        cx.set_exit(key("SUCCESS"));
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !self.started {
            cx.set_exit(key("FAILURE"));
            return true;
        }
        let code = ask(cx, Query::ServerOpError) as u32;
        if code != 0 {
            self.error(cx, code);
            return true;
        }
        if ask(cx, Query::ServerOpState) == 2 {
            if self.state == 0 {
                // 0x7400b0: Connect finished; the requested operation is sent next.
                self.state = OP_NONE;
                if self.requested != 0 {
                    self.started = self.start(cx, self.requested);
                }
                cx.host.request(Request::ServerOpReport { code: 0 });
            } else if self.state < OP_NONE {
                cx.host.request(Request::ServerOpHandle { op: self.state });
                self.state = ask(cx, Query::ServerOpNextState { state: self.state }) as u32;
            }
            if self.state == OP_NONE {
                cx.set_exit(key("SUCCESS"));
                return true;
            }
        }
        false
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        let state = ask(cx, Query::ServerOpState);
        if state != 2 && state != 0 {
            cx.host.request(Request::ServerOpCancel);
        }
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        let state = if self.acked { 0 } else { ask(cx, Query::ServerOpState) };
        if state == 2 {
            cx.host.request(Request::ServerOpAck);
            self.acked = true;
        }
        state == 0 || self.acked
    }
}

/// The exit a sign-in control machine takes when its control is left: `ENTER_NEXT` → `EXIT_NEXT`, `ENTER_PREV` →
/// `EXIT_PREV`, anything else `EXIT_NEXT` (0x7732ae).
fn control_exit(cx: &Cx) -> u64 {
    if cx.entry_point() == key("ENTER_PREV") { key("EXIT_PREV") } else { key("EXIT_NEXT") }
}

/// "Sign In Control" (`SIGNIN_CNTRL`, factory 0x77c720, vtable 0xfd9d48, 0x48 B): a control of the sign-in form
/// (`G+0x2f3e0` +0x10c8) named by `DATACONTEXT` (when not empty). Enter (0x773190) shows it
/// ([`Request::SignInFormActive`], `FUN_00884d40`), exit (0x773300) hides it. The update (0x7731e0) runs while the
/// control's two expressions ([`Query::SignInControlActive`] `FUN_00884fc0`, [`Query::SignInControlVisible`]
/// `FUN_00884ec0`) hold; when one fails it exits through [`control_exit`]. Which of the two is "active" and which
/// "visible" is not settled (model decision of the names; both are binding expressions of the control, +0xc).
pub struct SignInControl;

impl Behaviour for SignInControl {
    fn on_enter(&mut self, cx: &mut Cx) {
        let context = cx.param_text("DATACONTEXT");
        if !context.is_empty() {
            cx.host.request(Request::SignInFormActive { context, active: true });
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let context = cx.param_text("DATACONTEXT");
        if !context.is_empty()
            && (!ask_bool(cx, Query::SignInControlActive { context: context.clone() })
                || !ask_bool(cx, Query::SignInControlVisible { context }))
        {
            let exit = control_exit(cx);
            cx.set_exit(exit);
            return true;
        }
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        let context = cx.param_text("DATACONTEXT");
        if !context.is_empty() {
            cx.host.request(Request::SignInFormActive { context, active: false });
        }
    }
}

/// "Edit Text Is Empty" (`SI_EDT_EMPT`, factory 0x77c8b0, vtable 0xfd9dfc, 0x48 B): the update (0x772120) looks at
/// the text of the edit control `DATACONTEXT` ([`Query::SignInEditEmpty`]) and exits through `TRUE` (empty) or
/// `FALSE`, setting the control's empty flag (+0x23c, [`Request::SignInEditEmptyFlag`]).
pub struct EditTextEmpty;

impl Behaviour for EditTextEmpty {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let context = cx.param_text("DATACONTEXT");
        let empty = ask_bool(cx, Query::SignInEditEmpty { context: context.clone() });
        cx.set_exit(if empty { TRUE } else { FALSE });
        cx.host.request(Request::SignInEditEmptyFlag { context, empty });
        true
    }
}

/// "Keyboard Entry" (`HSM_KEYB_ENT`, factory 0x77c7c0, vtable 0xfd9d90, 0x48 B; slot 9 0x772880 sets the form's
/// `FLOW`): enter (0x7722b0) opens the on-screen keyboard (G+0x2f065) on the control `DATACONTEXT` with its
/// text, title and description ([`Request::OpenKeyboard`]); the flags come from `ISPASSWORD` (4), else
/// `ISEMAIL` (2), else `ISPRODKEY` (0x20), else `ISGAMERTAG` (8), each read as "true". The update (0x7724f0)
/// waits for the keyboard's result (2 cancelled, 3 accepted, [`Query::KeyboardResult`]), stores the text when it
/// was accepted, closes the keyboard and exits through `EXIT`.
pub struct KeyboardEntry;

impl Behaviour for KeyboardEntry {
    fn on_enter(&mut self, cx: &mut Cx) {
        let flags = if param_true(cx, "ISPASSWORD") {
            4
        } else if param_true(cx, "ISEMAIL") {
            2
        } else if param_true(cx, "ISPRODKEY") {
            0x20
        } else if param_true(cx, "ISGAMERTAG") {
            8
        } else {
            0
        };
        let context = cx.param_text("DATACONTEXT");
        cx.host.request(Request::OpenKeyboard { context, flags });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let result = ask(cx, Query::KeyboardResult);
        if result != 2 && result != 3 {
            return false;
        }
        let context = cx.param_text("DATACONTEXT");
        cx.host.request(Request::KeyboardStore { context, accepted: result == 3 });
        cx.host.request(Request::CloseKeyboard);
        cx.set_exit(EXIT);
        true
    }
}

/// "First Party Sign In" (`SI_FIRST_PRT`, factory 0x77ca00, vtable 0xfd9eb8, 0x50 B): enter (0x7721d0) notes whether
/// the first party is not signed in (`FUN_004c2030`, a constant true on the PC: [`Query::FirstPartySignedIn`]);
/// the update (0x7721f0) waits while that was so, else exits through `SUCCESS` when signed in, `FAILURE` if not.
#[derive(Default)]
pub struct FirstPartySignIn {
    waiting: bool,
}

impl Behaviour for FirstPartySignIn {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.waiting = !ask_bool(cx, Query::FirstPartySignedIn);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.waiting {
            return false;
        }
        let exit = if ask_bool(cx, Query::FirstPartySignedIn) { key("SUCCESS") } else { key("FAILURE") };
        cx.set_exit(exit);
        true
    }
}

/// The string sets of a form state (the shared part of `SI_ADD_CNTRL` and `SI_SHOW_ERR`): the enter collects them
/// (0x7730c0 over `LIST` / `ITEMS` and `DATE` / `MONTHS`, 0x7735f0 over `ERRORLIST`) and starts their load; the
/// enter update waits for it (0x772930 / 0x773350); the exit begin starts the unload (0x771030 / 0x772600) and the
/// exit update waits for it (0x771120 / 0x7726f0).
#[derive(Default)]
struct FormStrings {
    count: i64,
}

impl FormStrings {
    fn enter(&mut self, cx: &mut Cx) {
        let state = cx.state().id;
        self.count = ask(cx, Query::SignInFormStringSets { state }) as i64;
        if self.count > 0 {
            cx.host.request(Request::SignInFormLoadStrings { state });
        }
    }

    fn entered(&self, cx: &mut Cx) -> bool {
        self.count == 0 || !ask_bool(cx, Query::SignInFormBusy { unload: false })
    }

    fn exit_begin(&self, cx: &mut Cx) {
        if self.count > 0 {
            let state = cx.state().id;
            cx.host.request(Request::SignInFormUnloadStrings { state });
        }
    }

    fn exited(&mut self, cx: &mut Cx) -> bool {
        if self.count > 0 && ask_bool(cx, Query::SignInFormBusy { unload: true }) {
            return false;
        }
        self.count = 0;
        true
    }
}

/// "Add Controls" (`SI_ADD_CNTRL`, factory 0x77fb60, vtable 0xfdc0cc, 0xf8 B): builds a sign-in screen from the
/// state's parameters. Enter (0x773720) loads the string sets of its lists; once running (slot 2, 0x773870) the
/// form `SCREEN` (`FUN_00883750`) gets the controls of `BACKGROUND`, `BOOLEAN`, `COMMAND`, `DATE`, `EDITTEXT`,
/// `LIST` and `LONGMESSAGE` (0x771150 / 0x771380 / 0x7716c0 / 0x771980 / 0x772aa0 / 0x772d80 / 0x771b60: the
/// engine's work, [`Request::SignInFormBuild`]); the update (0x770ca0) does nothing; the exit (0x7729c0)
/// removes the controls and the screen.
#[derive(Default)]
pub struct AddControls {
    strings: FormStrings,
}

impl Behaviour for AddControls {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.strings.enter(cx);
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        self.strings.entered(cx)
    }

    fn on_running(&mut self, cx: &mut Cx) {
        let (state, screen) = (cx.state().id, cx.param_text("SCREEN"));
        cx.host.request(Request::SignInFormBuild { state, screen });
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        self.strings.exit_begin(cx);
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        self.strings.exited(cx)
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        let screen = cx.param_text("SCREEN");
        cx.host.request(Request::SignInFormRemove { screen });
    }
}

/// "Show Sign In Error" (`SI_SHOW_ERR`, factory 0x77fbf0, vtable 0xfdc100, 0x110 B): the error screen of the
/// sign-in (`DATACONTEXT` = `SignInState.Error`, `ERRORLIST` the string set, `STDLAYOUT` / `BTNLAYOUT` the layer
/// sets). Enter (0x7738e0) loads the string sets; running (0x773f40) builds the screen from the error the binding
/// picks ([`Request::SignInErrorShow`]); the update (0x773a10) sets the layer set and handles the buttons' focus
/// per frame ([`Request::SignInErrorFrame`], the engine's work) and exits when a button is confirmed (0x773680),
/// through the exit point of that button ([`Query::SignInErrorChoice`]); the exit (0x772720) restores the layer
/// set and removes the screen, with the loading screen shown again unless `SHOWLOAD` is given and not "true".
#[derive(Default)]
pub struct ShowSignInError {
    strings: FormStrings,
}

impl Behaviour for ShowSignInError {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.strings.enter(cx);
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        self.strings.entered(cx)
    }

    fn on_running(&mut self, cx: &mut Cx) {
        let state = cx.state().id;
        cx.host.request(Request::SignInErrorShow { state });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let state = cx.state().id;
        cx.host.request(Request::SignInErrorFrame { state });
        let choice = ask(cx, Query::SignInErrorChoice);
        if choice != 0 {
            cx.set_exit(choice);
            return true;
        }
        false
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        self.strings.exit_begin(cx);
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        self.strings.exited(cx)
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        let show_load = cx.param("SHOWLOAD").is_none() || param_true(cx, "SHOWLOAD");
        cx.host.request(Request::SignInErrorClose { show_load });
    }
}

/// "Write Session Variable" (`WRITESESSION`, factory 0x77d5d0, vtable 0xfda484, 0x48 B): enter (0x747280) sets the
/// session variable `VARNAME` to `VARVALUE` when both parameters exist (`FUN_007e94f0` on G+0x2f458 +0x280:
/// [`Request::WriteSession`]); the update (0x741bf0, `FUN_00774dc0`) exits through `EXIT` when the parent
/// handles it.
pub struct WriteSession;

impl Behaviour for WriteSession {
    fn on_enter(&mut self, cx: &mut Cx) {
        if cx.param("VARNAME").is_some() && cx.param("VARVALUE").is_some() {
            let (name, value) = (cx.param_text("VARNAME"), cx.param_text("VARVALUE"));
            cx.host.request(Request::WriteSession { name, value });
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if cx.parent_handles(EXIT) {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// "Load String Set" (`LOADSTRSET`, factory 0x77f9f0, vtable 0xfdbe3c, 0x210 B): loads the string-set bundle
/// `UI\LANGUAGE\<language>\<StringSet>.BNDL` (the credits, the easy guide's strings) while it runs. Enter
/// (0x744110) reads `STRINGSET` (`atoi`); the enter update (0x74de80) starts the load ([`Request::LoadStringSet`]),
/// waits for it ([`Query::StringSetBusy`], status 2 / 3) and makes the set current
/// ([`Request::ActivateStringSet`], `FUN_0046fea0`); the update (0x73dde0) is 0; the exit update (0x7441a0) waits
/// for a load still running, drops the set ([`Request::UnloadStringSet`], `FUN_00472e00` and the unload) and
/// waits for the unload.
#[derive(Default)]
pub struct LoadStringSet {
    id: i64,
    /// +0x20c: 0 idle, 1 loading, 2 loaded, 3 unloading.
    phase: u8,
}

impl Behaviour for LoadStringSet {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.phase = 0;
        self.id = param_int(cx, "STRINGSET");
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        match self.phase {
            0 | 1 => {
                if self.phase == 0 {
                    cx.host.request(Request::LoadStringSet { id: self.id });
                    self.phase = 1;
                }
                if ask_bool(cx, Query::StringSetBusy { unload: false }) {
                    return false;
                }
                cx.host.request(Request::ActivateStringSet { id: self.id });
                self.phase = 2;
                true
            }
            2 => true,
            _ => false,
        }
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        match self.phase {
            0 => return true,
            1 => {
                if ask_bool(cx, Query::StringSetBusy { unload: false }) {
                    return false;
                }
                self.phase = 2;
            }
            2 => {}
            _ => {}
        }
        if self.phase == 2 {
            cx.host.request(Request::UnloadStringSet { id: self.id });
            self.phase = 3;
        }
        if ask_bool(cx, Query::StringSetBusy { unload: true }) {
            return false;
        }
        self.phase = 0;
        true
    }
}
