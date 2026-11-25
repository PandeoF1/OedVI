// No console window next to the launcher on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    oedvi_launcher_lib::run()
}
