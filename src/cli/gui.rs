//! `rsdiskwrite gui` (also the no-argument default).

use crate::cli::GuiArgs;
use crate::error::Result;
use crate::output::Output;

/// Opens the graphical interface and blocks until it closes.
pub fn run(args: GuiArgs, out: &Output) -> Result<()> {
    let (GuiArgs {}, _) = (args, out);
    crate::gui::run()
}
