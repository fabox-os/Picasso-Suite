use std::ffi::OsString;
use std::os::raw::*;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use x11rb::protocol::xproto::{self, ConnectionExt};

use super::atoms::AtomName::None as DndNone;
use super::atoms::*;
use super::{util, CookieResultExt, X11Error, XConnection};

#[derive(Debug, Clone, Copy)]
pub enum DndState {
    Accepted,
    Rejected,
}

/// Why a drag offered no file we could use (Picasso Suite change: upstream gave up on the whole
/// list for one line it could not read; now only a list with no usable line fails).
#[derive(Debug)]
pub enum DndDataParseError {
    EmptyData,
    NoUsableFile(#[allow(dead_code)] String),
}

pub struct Dnd {
    xconn: Arc<XConnection>,
    // Populated by XdndEnter event handler
    pub version: Option<c_long>,
    pub type_list: Option<Vec<xproto::Atom>>,
    // Populated by XdndPosition event handler
    pub source_window: Option<xproto::Window>,
    // Populated by SelectionNotify event handler (triggered by XdndPosition event handler)
    pub result: Option<Result<Vec<PathBuf>, DndDataParseError>>,
    // Picasso Suite change: set by the XdndDrop handler when the drop came before the list of files
    // did (the compositor is still reading it from the file manager, which takes longer for a large
    // batch): the window the drag came from, to be answered once the list is here.
    pub pending_drop: Option<xproto::Window>,
}

impl Dnd {
    pub fn new(xconn: Arc<XConnection>) -> Result<Self, X11Error> {
        Ok(Dnd {
            xconn,
            version: None,
            type_list: None,
            source_window: None,
            result: None,
            pending_drop: None,
        })
    }

    pub fn reset(&mut self) {
        self.version = None;
        self.type_list = None;
        self.source_window = None;
        self.result = None;
        self.pending_drop = None;
    }

    pub unsafe fn send_status(
        &self,
        this_window: xproto::Window,
        target_window: xproto::Window,
        state: DndState,
    ) -> Result<(), X11Error> {
        let atoms = self.xconn.atoms();
        let (accepted, action) = match state {
            DndState::Accepted => (1, atoms[XdndActionPrivate]),
            DndState::Rejected => (0, atoms[DndNone]),
        };
        self.xconn
            .send_client_msg(target_window, target_window, atoms[XdndStatus] as _, None, [
                this_window,
                accepted,
                0,
                0,
                action as _,
            ])?
            .ignore_error();

        Ok(())
    }

    pub unsafe fn send_finished(
        &self,
        this_window: xproto::Window,
        target_window: xproto::Window,
        state: DndState,
    ) -> Result<(), X11Error> {
        let atoms = self.xconn.atoms();
        let (accepted, action) = match state {
            DndState::Accepted => (1, atoms[XdndActionPrivate]),
            DndState::Rejected => (0, atoms[DndNone]),
        };
        self.xconn
            .send_client_msg(target_window, target_window, atoms[XdndFinished] as _, None, [
                this_window,
                accepted,
                action as _,
                0,
                0,
            ])?
            .ignore_error();

        Ok(())
    }

    pub unsafe fn get_type_list(
        &self,
        source_window: xproto::Window,
    ) -> Result<Vec<xproto::Atom>, util::GetPropertyError> {
        let atoms = self.xconn.atoms();
        self.xconn.get_property(
            source_window,
            atoms[XdndTypeList],
            xproto::Atom::from(xproto::AtomEnum::ATOM),
        )
    }

    pub unsafe fn convert_selection(&self, window: xproto::Window, time: xproto::Timestamp) {
        let atoms = self.xconn.atoms();
        self.xconn
            .xcb_connection()
            .convert_selection(
                window,
                atoms[XdndSelection],
                atoms[TextUriList],
                atoms[XdndSelection],
                time,
            )
            .expect_then_ignore_error("Failed to send XdndSelection event")
    }

    pub unsafe fn read_data(
        &self,
        window: xproto::Window,
    ) -> Result<Vec<c_uchar>, util::GetPropertyError> {
        let atoms = self.xconn.atoms();
        self.xconn.get_property(window, atoms[XdndSelection], atoms[TextUriList])
    }

    pub fn parse_data(&self, data: &mut [c_uchar]) -> Result<Vec<PathBuf>, DndDataParseError> {
        if data.is_empty() {
            return Err(DndDataParseError::EmptyData);
        }
        let (found, mut left_out) = super::uri_list::uri_list_paths(data);
        let mut path_list = Vec::with_capacity(found.len());
        for bytes in found {
            let path = PathBuf::from(OsString::from_vec(bytes));
            match Path::new(&path).canonicalize() {
                Ok(path) => path_list.push(path),
                // A path that cannot be resolved but is there (a folder that cannot be listed, say)
                // is still the file that was dragged.
                Err(_) if path.exists() => path_list.push(path),
                Err(e) => left_out.push(format!("{}: {e}", path.display())),
            }
        }
        if !left_out.is_empty() {
            tracing::warn!(
                "drag and drop: {} of the dragged files were left out: {}",
                left_out.len(),
                left_out.join("; ")
            );
        }
        if path_list.is_empty() {
            return Err(DndDataParseError::NoUsableFile(left_out.join("; ")));
        }
        Ok(path_list)
    }
}
