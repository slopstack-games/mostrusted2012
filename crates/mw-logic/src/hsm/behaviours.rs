//! The ported behaviour types. Each names its factory and vtable in the original (the factory allocates the
//! instance and sets the vtable; docs/decomp.md "The HSM interpreter" lists the slots). Binding reads go through
//! [`super::Host`]: a binding that does not resolve counts as false / empty, like the original's evaluators.

use mw_formats::hsm::key;

use super::{Behaviour, Cx, EXIT, EXITINVALID, Key, Registry};

/// `TRUE` (`DAT_0117aac0`).
pub const TRUE: Key = 0xbec0_6eaf_9900_0000;
/// `FALSE` (`DAT_0117a960`).
pub const FALSE: Key = 0x6ac1_7617_bea0_0000;

/// The `BINDING` parameter id of most binding behaviours: "BINDING", a NUL, then "PATH" (0x54a48941b5db2e54,
/// read from the exe's constants `DAT_0117a570` / `0117a150` / `0117a218` / `0117a288` / `0117a050`; the data's
/// 484 `BINDING` parameters carry it). `BINDINGTIMED` and `RUNSUBNET` use the plain `BINDING`
/// (`DAT_0117a778`, `DAT_0117abb8`). Parameter lookups compare all 64 bits (`FUN_00774890`).
pub const BINDING_PATH: Key = 0x54a4_8941_b5db_2e54;

/// Registers every ported behaviour.
pub fn register(r: &mut Registry) {
    r.register_fn("HSB_RUNCHILD", |_| RunChild);
    r.register_fn("HSB_PLCHLDR", |_| Placeholder);
    r.register_fn("WAITCHILDREN", |_| WaitChildren);
    r.register_fn("CALLHSMHOOK", |_| CallHook::default());
    r.register_fn("CALLHSMHOOKS", |_| CallHookScoped);
    r.register_fn("HSMHOOKEXIT", |_| HookExit::default());
    r.register_fn("EXITBINDTRUE", |_| ExitWhenBindingTrue::default());
    r.register_fn("HSB_SEL_EX_B", |_| SelectExitFromBinding);
    r.register_fn("HSM_B_TRUE", |_| IsBindingTrue);
    r.register_fn("HSB_B_NOT_0", |_| IsBindingNotNull);
    r.register_fn("HSB_EX_BI_CH", |_| ExitWhenBindingChanges::default());
    r.register_fn("BINDINGTIMED", |_| BindingTrueForTime::default());
    r.register_fn("BDELAY", |_| Delay::default());
    r.register_fn("WAIT4TIME", |_| WaitForTime::default());
    r.register_fn("RUNSUBNET", |_| RunSubNetwork::default());
    r.register_fn("LOCALECHECK", |_| LocaleCheck);
    r.register_fn("BWAIT4GFX", |_| WaitForGraphics);
    r.register_fn("HSB_WAIT4LS", |_| WaitForLoadingScreen { showing: true });
    r.register_fn("HSB_WAIT4NL", |_| WaitForLoadingScreen { showing: false });
}

/// "Run Child Behaviour" (`HSB_RUNCHILD`, factory 0x77ab20, vtable 0xfd9194): runs its children until one of
/// their exit points exits it (update 0x73ed50 returns 0).
pub struct RunChild;
impl Behaviour for RunChild {}

/// `HSB_PLCHLDR` (factory 0x77aa80, vtable 0xfd914c): exits through `EXIT` on its first update (0x73e670); the
/// fallback of states whose behaviour is not registered.
pub struct Placeholder;
impl Behaviour for Placeholder {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "HSM.Wait For Children" (`WAITCHILDREN`, factory 0x77aad0, vtable 0xfd9170): exits through `EXIT` once it
/// has no children left (0x73e690).
pub struct WaitChildren;
impl Behaviour for WaitChildren {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if cx.has_children() {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "CallHSMHook" (`CALLHSMHOOK`, factory 0x77a990, vtable 0xfd90e0): on enter (0x76f0f0) the hook named by
/// `FUNCTION`; on its first update (0x76e420) calls it with the data definition's `Params` and exits through
/// whatever the hook returns ([`EXITINVALID`] for a missing hook).
#[derive(Default)]
pub struct CallHook {
    function: String,
}
impl Behaviour for CallHook {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.function = cx.param_text("FUNCTION");
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        let r = cx.call_hook(&self.function, "Params");
        cx.set_exit(r);
        true
    }
}

/// "CallHSMHookScoped" (`CALLHSMHOOKS`, factory 0x77a9e0, vtable 0xfd9104): calls `ONENTERHOOK` with
/// `EnterParams` on enter (0x76f120) and `ONEXITHOOK` with `ExitParams` when finished (0x76f1a0), when named;
/// the results are ignored; runs its children in between.
pub struct CallHookScoped;
impl Behaviour for CallHookScoped {
    fn on_enter(&mut self, cx: &mut Cx) {
        let name = cx.param_text("ONENTERHOOK");
        if !name.is_empty() {
            cx.call_hook(&name, "EnterParams");
        }
    }
    fn on_exit(&mut self, cx: &mut Cx) {
        let name = cx.param_text("ONEXITHOOK");
        if !name.is_empty() {
            cx.call_hook(&name, "ExitParams");
        }
    }
}

/// "Select Exit From HSM Hook" (`HSMHOOKEXIT`, factory 0x77aa30, vtable 0xfd9128): calls `FUNCTION` with
/// `Params` every update (0x76e480) until it returns something other than [`EXITINVALID`], and exits through it.
#[derive(Default)]
pub struct HookExit {
    function: String,
}
impl Behaviour for HookExit {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.function = cx.param_text("FUNCTION");
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        let r = cx.call_hook(&self.function, "Params");
        if r == EXITINVALID {
            return false;
        }
        cx.set_exit(r);
        true
    }
}

/// "Exit When Binding Is True" (`EXITBINDTRUE`, factory 0x55f790, vtable 0xfab66c): parses `BINDING` on enter
/// (0x742fc0); every update (0x743000), when the binding parsed, exits through `EXIT` once it reads true. Runs
/// its children meanwhile.
#[derive(Default)]
pub struct ExitWhenBindingTrue {
    binding: String,
}
impl Behaviour for ExitWhenBindingTrue {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.binding = cx.param_id_text(BINDING_PATH);
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        // Model: "parsed" (+0xbc > 0 after FUN_00a50650) = a non-empty path, until the binding parser is ported.
        if self.binding.is_empty() || cx.host.binding_bool(&self.binding) != Some(true) {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "Select Exit Point From Binding" (`HSB_SEL_EX_B`, factory 0x77bc90, vtable 0xfd9860; update 0x7457d0): exits
/// at once through the exit point named by the binding's text value, or through `ONFAILEXIT` when it does not
/// resolve.
pub struct SelectExitFromBinding;
impl Behaviour for SelectExitFromBinding {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let binding = cx.param_id_text(BINDING_PATH);
        let exit = match cx.host.binding_string(&binding) {
            Some(text) => key(&text),
            None => key(&cx.param_text("ONFAILEXIT")),
        };
        cx.set_exit(exit);
        true
    }
}

/// "HSM.Is Binding True" (`HSM_B_TRUE`, factory 0x77bd30, vtable 0xfd98a8; update 0x7459b0): exits at once
/// through `TRUE` or `FALSE` (also when it does not resolve).
pub struct IsBindingTrue;
impl Behaviour for IsBindingTrue {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let binding = cx.param_id_text(BINDING_PATH);
        let v = cx.host.binding_bool(&binding) == Some(true);
        cx.set_exit(if v { TRUE } else { FALSE });
        true
    }
}

/// "Is Binding Not Null" (`HSB_B_NOT_0`, factory 0x77bce0, vtable 0xfd9884; update 0x7458a0): exits at once
/// through `TRUE` when the binding's text is neither empty nor "0", else `FALSE`.
pub struct IsBindingNotNull;
impl Behaviour for IsBindingNotNull {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let binding = cx.param_id_text(BINDING_PATH);
        let text = cx.host.binding_string(&binding).unwrap_or_default();
        cx.set_exit(if !text.is_empty() && text != "0" { TRUE } else { FALSE });
        true
    }
}

/// "Exit When Binding Changes" (`HSB_EX_BI_CH`, factory 0x77bd80, vtable 0xfd98cc): the first update that
/// resolves the binding (0x74b840 → 0x742f60) keeps its value; later updates exit through `EXIT` once a resolved
/// value differs.
#[derive(Default)]
pub struct ExitWhenBindingChanges {
    binding: String,
    first: Option<i64>,
}
impl Behaviour for ExitWhenBindingChanges {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.binding = cx.param_id_text(BINDING_PATH);
        self.first = None;
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        let Some(first) = self.first else {
            self.first = cx.host.binding_int(&self.binding);
            return false;
        };
        if self.binding.is_empty() {
            return false;
        }
        match cx.host.binding_int(&self.binding) {
            Some(v) if v != first => {
                cx.set_exit(EXIT);
                true
            }
            _ => false,
        }
    }
}

/// "BindingIsTrueForTime" (`BINDINGTIMED`, factory 0x779c60, vtable 0xfd8ee8; enter 0x7420d0, update 0x742140):
/// counts `TIME` seconds down on the game clock while the binding reads true (not while that clock is stopped),
/// restarts the count when it reads false, exits through `EXIT` at 0.
#[derive(Default)]
pub struct BindingTrueForTime {
    binding: String,
    time: f32,
    left: f32,
}
impl Behaviour for BindingTrueForTime {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.binding = cx.param_text("BINDING");
        self.time = cx.param_f32("TIME");
        self.left = self.time;
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.binding.is_empty() {
            return false;
        }
        if cx.host.binding_bool(&self.binding) != Some(true) {
            self.left = self.time;
            return false;
        }
        let Some(dt) = cx.clock.game else { return false };
        self.left -= dt;
        if self.left <= 0.0 {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// `BDELAY` (factory 0x779280, vtable 0xfd8d80): waits `ENTERTIME` s while entering (0x73bd40), `UPDATETIME` s
/// while running before exiting through `EXIT` (0x73bd80), `LEAVETIME` s while exiting (0x73bdd0), each on the
/// frame clock (enter 0x742770 reads the three parameters).
#[derive(Default)]
pub struct Delay {
    times: [f32; 3],
    elapsed: [f32; 3],
}
impl Delay {
    fn wait(&mut self, i: usize, dt: f32) -> bool {
        self.elapsed[i] += dt;
        self.elapsed[i] >= self.times[i]
    }
}
impl Behaviour for Delay {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.times = [cx.param_f32("ENTERTIME"), cx.param_f32("UPDATETIME"), cx.param_f32("LEAVETIME")];
        self.elapsed = [0.0; 3];
    }
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        self.wait(0, cx.clock.frame)
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.wait(1, cx.clock.frame) {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        self.wait(2, cx.clock.frame)
    }
}

/// Ticks per second of the timers (`.rdata` 0x00f9f604 = 3e6).
const TICKS: f64 = 3.0e6;

/// "Wait For Time" (`WAIT4TIME`, factory 0x5a23f0, vtable 0xfae1a4): `TIME` s as 3 MHz ticks (enter 0x758950),
/// counted down by the frame clock (update 0x741ac0) until less than one microsecond (ticks / 3 ≤ 0) is left, then
/// `EXIT`. With `REGBINDING` true (the default) the timer is also published as the view model
/// `CurrentHSMTimer` (not ported: open).
#[derive(Default)]
pub struct WaitForTime {
    left: i64,
}
impl Behaviour for WaitForTime {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.left = (super::atof(&cx.param_text("TIME")) * TICKS) as i64;
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        self.left -= (cx.clock.frame as f64 * TICKS) as i64;
        if self.left / 3 <= 0 {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// "Run Sub Network" (`RUNSUBNET`, factory 0x77dd50, vtable 0xfda810): starts the top-level state whose
/// instance id is the `BINDING`'s value, or `TOPSTATE` when that is empty (0x745710), as its own machine
/// (0x7500a0), waits for it (0x73f1e0) and exits through the machine's exit point when its own parent handles
/// that, else through `EXIT`; an exit from outside first asks the machine to exit and waits for it
/// (0x73f170, 0x73f1b0).
#[derive(Default)]
pub struct RunSubNetwork {
    serial: Option<u32>,
}
impl Behaviour for RunSubNetwork {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        let binding = cx.param_text("BINDING");
        let instance = if binding.is_empty() {
            cx.param_text("TOPSTATE").trim().parse::<i64>().unwrap_or(0)
        } else {
            cx.host.binding_int(&binding).unwrap_or(0)
        };
        self.serial = cx.start_machine(instance as u32);
        true
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        let Some(serial) = self.serial else {
            cx.set_exit(EXIT);
            return true;
        };
        if cx.machine_running(serial) {
            return false;
        }
        match cx.machine_result(serial) {
            Some(exit) if cx.parent_handles(exit) => cx.set_exit(exit),
            _ => cx.set_exit(EXIT),
        }
        true
    }
    fn on_exit_begin(&mut self, cx: &mut Cx) {
        if let Some(s) = self.serial
            && cx.machine_running(s)
            && !cx.machine_exit_requested(s)
        {
            cx.request_machine_exit(s);
        }
    }
    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if self.serial.is_some_and(|s| cx.machine_running(s)) {
            return false;
        }
        self.serial = None;
        true
    }
}

/// "Locale Check" (`LOCALECHECK`, factory 0x779ff0, vtable 0xfd8fc0): on PC exits through `EXIT` at once
/// (0x73e0a0); the console builds pick a language-select exit point instead.
pub struct LocaleCheck;
impl Behaviour for LocaleCheck {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Wait For Graphics Prepare Behaviour" (`BWAIT4GFX`, factory 0x7792e0, vtable 0xfd8da4): exits through `EXIT`
/// at once (0x73e0a0-like 0x741720) but finishes only once the graphics are prepared (exit update 0x741740).
pub struct WaitForGraphics;
impl Behaviour for WaitForGraphics {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        cx.host.graphics_prepared()
    }
}

/// "Wait For Loading Screen Showing" (`HSB_WAIT4LS`, factory 0x77c1f0, vtable 0xfd9adc, update 0x741c10) and
/// "... Not Showing" (`HSB_WAIT4NL`, factory 0x77c1a0, vtable 0xfd9ab8, update 0x741c50): exit through `EXIT`
/// once the loading screen shows / no longer shows; run their children meanwhile.
pub struct WaitForLoadingScreen {
    showing: bool,
}
impl Behaviour for WaitForLoadingScreen {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if cx.host.loading_screen_showing() != self.showing {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// A stand-in for a behaviour that is not ported: exits through a fixed exit point on its first update, or
/// runs its children forever (`None`). For headless drivers and tests; not a port of anything.
pub struct Stub {
    pub exit: Option<Key>,
}
impl Behaviour for Stub {
    fn update(&mut self, cx: &mut Cx) -> bool {
        match self.exit {
            Some(e) => {
                cx.set_exit(e);
                true
            }
            None => false,
        }
    }
}
