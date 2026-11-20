//! `rsdiskwrite gui` (also the no-argument default).

use crate::cli::GuiArgs;
use crate::error::Result;
use crate::output::Output;

/// When the console was created just for this process (double-click launch), free it so
/// no stray console window sits behind the GUI. A shell-launched run is left alone.
fn release_own_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut pids = [0u32; 2];
    // SAFETY: the buffer is valid for the stated length of 2 entries.
    let count = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), 2) };
    if count == 1 {
        // SAFETY: plain Win32 call detaching this process from its console.
        unsafe { FreeConsole() };
    }
}

/// Opens the graphical interface and blocks until it closes.
pub fn run(args: GuiArgs, out: &Output) -> Result<()> {
    let (GuiArgs {}, _) = (args, out);
    release_own_console();
    crate::gui::run()
}
