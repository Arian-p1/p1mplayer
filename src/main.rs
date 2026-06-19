// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod library;
mod media;
mod model;
mod palette;
mod queue;
mod store;

use app::{Controller, MainWindow};
use slint::{ComponentHandle, Timer, TimerMode};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // On Wayland the compositor/taskbar derives a window's icon from its application
    // id (matched to an installed `p1mplayer.desktop` + themed icon), and ignores the
    // in-window `icon` property; X11 uses WM_CLASS similarly. Set a stable app id so
    // the right icon is picked up. (Linux/BSD only; a no-op elsewhere.)
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use slint::winit_030::winit::platform::wayland::WindowAttributesExtWayland;
        if let Err(e) = slint::BackendSelector::new()
            .with_winit_window_attributes_hook(|attrs| attrs.with_name("p1mplayer", "p1mplayer"))
            .select()
        {
            eprintln!("p1mplayer: could not set application id: {e}");
        }
    }

    // Load persisted state. The full library is restored instantly from disk; we
    // refresh it from the saved folders in the background after the window opens.
    let state = store::load();

    let ui = MainWindow::new()?;
    let controller = Rc::new(RefCell::new(Controller::new(state)));

    // Initial paint (instant), then kick a non-blocking rescan of saved folders.
    controller.borrow_mut().refresh_lists(&ui);
    controller.borrow_mut().start_startup_rescan(&ui);

    wire_callbacks(&ui, &controller);

    // Drive progress + auto-advance.
    let tick_timer = Timer::default();
    {
        let weak = ui.as_weak();
        let ctrl = controller.clone();
        tick_timer.start(TimerMode::Repeated, Duration::from_millis(400), move || {
            if let Some(ui) = weak.upgrade() {
                ctrl.borrow_mut().tick(&ui);
            }
        });
    }

    ui.run()?;

    // Persist on exit.
    controller.borrow().save();
    Ok(())
}

fn wire_callbacks(ui: &MainWindow, controller: &Rc<RefCell<Controller>>) {
    macro_rules! on {
        // callback with no args
        ($setter:ident, $method:ident) => {{
            let weak = ui.as_weak();
            let ctrl = controller.clone();
            ui.$setter(move || {
                if let Some(ui) = weak.upgrade() {
                    ctrl.borrow_mut().$method(&ui);
                }
            });
        }};
        // callback with one arg, transformed before forwarding
        ($setter:ident, $method:ident, $arg:ident => $conv:expr) => {{
            let weak = ui.as_weak();
            let ctrl = controller.clone();
            ui.$setter(move |$arg| {
                if let Some(ui) = weak.upgrade() {
                    ctrl.borrow_mut().$method($conv, &ui);
                }
            });
        }};
    }

    on!(on_add_folder, add_folder);
    on!(on_search, search, q => q.to_string());
    on!(on_show_library, show_library);
    on!(on_show_queue, show_queue);
    on!(on_play_pause, play_pause);
    on!(on_next_track, next_track);
    on!(on_prev_track, prev_track);
    on!(on_seek_forward, seek_forward);
    on!(on_seek_back, seek_back);
    on!(on_toggle_shuffle, toggle_shuffle);
    on!(on_cycle_repeat, cycle_repeat);
    on!(on_toggle_hide_duplicates, toggle_hide_duplicates);

    on!(on_select_playlist, select_playlist, id => id as usize);
    on!(on_play_track, play_track, id => id as usize);
    on!(on_queue_track, queue_track, id => id as usize);
    on!(on_add_track_to_playlist, add_track_to_playlist, id => id as usize);
    on!(on_remove_track_from_queue, remove_track_from_queue, id => id as usize);
    on!(on_delete_track_file, delete_track_file, id => id as usize);
    on!(on_play_queue_pos, play_queue_pos, pos => pos as usize);
    on!(on_remove_queue_pos, remove_queue_pos, pos => pos as usize);
    on!(on_seek_to, seek_to, v => v);
    on!(on_set_volume, set_volume, v => v);

    // Two-arg callback: (track id, playlist id) → add the track to that playlist.
    {
        let weak = ui.as_weak();
        let ctrl = controller.clone();
        ui.on_add_track_to_playlist_id(move |track_id, playlist_id| {
            if let Some(ui) = weak.upgrade() {
                ctrl.borrow_mut()
                    .add_track_to_playlist_id(track_id as usize, playlist_id as usize, &ui);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let ctrl = controller.clone();
        ui.on_new_playlist(move |name| {
            if let Some(ui) = weak.upgrade() {
                ctrl.borrow_mut().new_playlist(name.to_string(), &ui);
            }
        });
    }
}
