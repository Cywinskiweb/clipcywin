#![windows_subsystem = "windows"]
#![allow(clippy::too_many_arguments)]

mod app;
mod clipboard;
mod db;
mod dnd;
mod gfx;
mod i18n;
mod images;
mod input;
mod model;
mod msg;
mod privacy;
mod settings;
mod settings_ui;
mod system;
mod ui;
mod util;

use std::time::Instant;

fn main() {
    let start = Instant::now();
    let autostart = std::env::args().any(|a| a == "--autostart");

    let Some(_instance) = system::single_instance::acquire() else {
        return;
    };

    let paths = settings::paths::Paths::resolve();
    let settings = settings::Settings::load(&paths.settings);
    i18n::set(&settings.general.language);
    util::log::init(&paths.logs.join("clipcywin.log"), settings.log_level());
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
        log::logger().flush();
    }));
    log::info!("clipcywin {} starting", env!("CARGO_PKG_VERSION"));

    let _com = system::com::init_sta();
    if let Err(e) = ui::win::register_classes() {
        log::error!("register classes: {e}");
        return;
    }
    app::run(paths, settings, autostart, start);
}
