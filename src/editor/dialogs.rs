//! Native file dialogs, shown outside of the editor's frame callback.
//!
//! Modal dialogs spin a nested event loop. Running one from inside baseview's frame callback lets
//! baseview dispatch events to the window handler it is still borrowing, which panics on macOS.
//! On macOS dialogs are therefore queued on the main dispatch queue, which runs them after the
//! current callback has returned. Elsewhere they run on their own thread. The editor picks up the
//! result on a later frame.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::{kit, loader};

pub enum Request {
    Sample {
        pad: usize,
        layer: usize,
        rename: bool,
    },
    RelinkFolder,
    LoadKit,
    SaveKit {
        file_name: String,
    },
    ExportMidi {
        file_name: String,
    },
    RecordingsFolder,
}

pub enum Outcome {
    Sample {
        pad: usize,
        layer: usize,
        rename: bool,
        path: PathBuf,
    },
    RelinkFolder(PathBuf),
    LoadKit(PathBuf),
    SaveKit(PathBuf),
    ExportMidi(PathBuf),
    RecordingsFolder(PathBuf),
}

#[derive(Default)]
pub struct Dialogs {
    open: Arc<AtomicBool>,
    outcome: Arc<Mutex<Option<Outcome>>>,
}

impl Dialogs {
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Shows a dialog unless one is already open.
    pub fn show(&self, request: Request, start_dir: Option<PathBuf>) {
        if self.open.swap(true, Ordering::AcqRel) {
            return;
        }

        let open = self.open.clone();
        let outcome = self.outcome.clone();
        let work = move || {
            *outcome.lock() = run(request, start_dir);
            open.store(false, Ordering::Release);
        };

        #[cfg(target_os = "macos")]
        dispatch2::DispatchQueue::main().exec_async(work);
        #[cfg(not(target_os = "macos"))]
        std::thread::spawn(work);
    }

    pub fn take_outcome(&self) -> Option<Outcome> {
        self.outcome.lock().take()
    }
}

fn run(request: Request, start_dir: Option<PathBuf>) -> Option<Outcome> {
    let mut dialog = rfd::FileDialog::new();
    if let Some(dir) = start_dir {
        dialog = dialog.set_directory(dir);
    }

    match request {
        Request::Sample { pad, layer, rename } => dialog
            .add_filter("Audio", loader::SUPPORTED_EXTENSIONS)
            .pick_file()
            .map(|path| Outcome::Sample {
                pad,
                layer,
                rename,
                path,
            }),
        Request::RelinkFolder => dialog.pick_folder().map(Outcome::RelinkFolder),
        Request::LoadKit => dialog
            .add_filter("Klopfer Kit or SFZ", &[kit::EXTENSION, "sfz"])
            .pick_file()
            .map(Outcome::LoadKit),
        Request::SaveKit { file_name } => dialog
            .add_filter("Klopfer Kit", &[kit::EXTENSION])
            .set_file_name(file_name)
            .save_file()
            .map(Outcome::SaveKit),
        Request::ExportMidi { file_name } => dialog
            .add_filter("MIDI File", &["mid"])
            .set_file_name(file_name)
            .save_file()
            .map(Outcome::ExportMidi),
        Request::RecordingsFolder => dialog.pick_folder().map(Outcome::RecordingsFolder),
    }
}
