//! The graphics card: decoding video on it (what `ffmpeg -hwaccel auto` does) and handing pictures
//! to the VA-API encoders of Linux.
//!
//! ffmpeg-next has no safe wrapper for FFmpeg's hardware devices, so this is the one module of the
//! program with `unsafe` code. It only passes FFmpeg's own pointers back to FFmpeg, checks every
//! pointer and return code, and owns each reference it takes until `Drop` gives it back.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use ff::ffi;
use ff::format::Pixel;
use ffmpeg_next as ff;

/// Whether video is decoded on the graphics card where it can be (System Preferences > Decode
/// Options). It belongs to the computer, not to a project. On by default, as in 0.2.0.
static HARDWARE_DECODING: AtomicBool = AtomicBool::new(true);

/// Turns hardware decoding on or off for the decoders opened from now on.
pub fn set_hardware_decoding(on: bool) {
    HARDWARE_DECODING.store(on, Ordering::Relaxed);
}

/// Whether decoders opened now use the graphics card. `PICASSO_NO_HWACCEL` turns it off for one
/// run whatever the preference says.
pub fn hardware_decoding() -> bool {
    HARDWARE_DECODING.load(Ordering::Relaxed) && std::env::var_os("PICASSO_NO_HWACCEL").is_none()
}

/// The codecs worth decoding on the card. Intra-frame codecs such as ProRes or DNxHR are left to
/// the processor, which decodes them fast and which most cards cannot help with.
const CODECS: [ff::codec::Id; 6] = [
    ff::codec::Id::H264,
    ff::codec::Id::HEVC,
    ff::codec::Id::AV1,
    ff::codec::Id::VP9,
    ff::codec::Id::MPEG2VIDEO,
    ff::codec::Id::VC1,
];

/// The kinds of device tried, best first. D3D11 and VA-API work with every vendor's card; CUDA is
/// NVIDIA's own, so it is only asked for when NVIDIA's driver is the one in charge.
fn device_kinds() -> &'static [ffi::AVHWDeviceType] {
    use ffi::AVHWDeviceType::*;
    if cfg!(windows) {
        &[
            AV_HWDEVICE_TYPE_D3D11VA,
            AV_HWDEVICE_TYPE_CUDA,
            AV_HWDEVICE_TYPE_DXVA2,
        ]
    } else if cfg!(target_os = "macos") {
        &[AV_HWDEVICE_TYPE_VIDEOTOOLBOX]
    } else if nvidia_driver() {
        &[AV_HWDEVICE_TYPE_VAAPI, AV_HWDEVICE_TYPE_CUDA]
    } else {
        &[AV_HWDEVICE_TYPE_VAAPI]
    }
}

/// Whether NVIDIA's driver is loaded, which is the only way a CUDA device can open. On a machine
/// with an Intel or AMD card (or with NVIDIA's card on the free driver) asking for CUDA fails with
/// "Cannot load libcuda.so.1" on stderr, which reads like a fault when it is only a card that was
/// never there.
fn nvidia_driver() -> bool {
    std::path::Path::new("/dev/nvidiactl").exists() || std::path::Path::new("/dev/nvidia0").exists()
}

/// A reference to an FFmpeg hardware device or frame pool.
struct BufferRef(*mut ffi::AVBufferRef);

// SAFETY: FFmpeg's buffer references count atomically, and the device contexts made here (D3D11,
// DXVA2, CUDA, VA-API, VideoToolbox) lock themselves when two threads use them.
unsafe impl Send for BufferRef {}

impl BufferRef {
    /// A new reference to the same buffer, for FFmpeg to own.
    fn new_ref(&self) -> *mut ffi::AVBufferRef {
        // SAFETY: `self.0` is a live reference (never null: see where it is made).
        unsafe { ffi::av_buffer_ref(self.0) }
    }
}

impl Drop for BufferRef {
    fn drop(&mut self) {
        // SAFETY: the reference is ours; unref sets the pointer to null.
        unsafe { ffi::av_buffer_unref(&mut self.0) }
    }
}

/// Opens a device of `kind` on the default card, or says why FFmpeg would not.
fn open_device_of(kind: ffi::AVHWDeviceType) -> Result<BufferRef, i32> {
    open_device_named(kind, None)
}

/// Opens a device of `kind` on the card called `name` (a render node such as
/// `/dev/dri/renderD129` for VA-API), or on the default card when there is no name.
fn open_device_named(kind: ffi::AVHWDeviceType, name: Option<&str>) -> Result<BufferRef, i32> {
    let name = match name.map(std::ffi::CString::new) {
        None => None,
        Some(Ok(c)) => Some(c),
        Some(Err(_)) => return Err(ffi::AVERROR(ffi::EINVAL)),
    };
    let mut device: *mut ffi::AVBufferRef = ptr::null_mut();
    // SAFETY: FFmpeg fills `device` on success and leaves it null otherwise; `name` outlives the
    // call.
    let made = unsafe {
        ffi::av_hwdevice_ctx_create(
            &mut device,
            kind,
            name.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            ptr::null_mut(),
            0,
        )
    };
    if made >= 0 && !device.is_null() {
        Ok(BufferRef(device))
    } else {
        Err(made)
    }
}

/// The graphics cards' render nodes (`/dev/dri/renderD128`, `renderD129`, ...) in order; none off
/// Linux. A computer with two cards (a processor's graphics and a separate card) has two, and the
/// default one is not always the card that can encode or decode a codec.
pub fn vaapi_nodes() -> Vec<String> {
    let mut nodes: Vec<String> = std::fs::read_dir("/dev/dri")
        .map(|dir| {
            dir.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
                .map(|e| e.path().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    nodes
}

/// A VA-API device on the default card, or else on the first render node that opens.
fn open_vaapi() -> Result<BufferRef, i32> {
    let kind = ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI;
    let first = open_device_of(kind);
    if first.is_ok() {
        return first;
    }
    for node in vaapi_nodes() {
        if let Ok(device) = open_device_named(kind, Some(&node)) {
            tracing::debug!("VA-API: the default card did not open, using {node}");
            return Ok(device);
        }
    }
    first
}

/// Where a VA-API encoder was found to work: the render node, and whether it needs the card's
/// low-power mode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VaapiChoice {
    /// `None`: the default card.
    pub node: Option<String>,
    pub low_power: bool,
}

static VAAPI_CHOICES: Mutex<Vec<(String, VaapiChoice)>> = Mutex::new(Vec::new());

/// Remembers where the encoder `name` works, for [`VaapiFrames::new`] and the export.
pub fn remember_vaapi_choice(name: &str, choice: VaapiChoice) {
    let mut all = VAAPI_CHOICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    all.retain(|(n, _)| n != name);
    all.push((name.to_string(), choice));
}

/// Where the encoder `name` was found to work; the default card without low power when it was
/// not checked.
pub fn vaapi_choice(name: &str) -> VaapiChoice {
    let all = VAAPI_CHOICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    all.iter()
        .find(|(n, _)| n == name)
        .map(|(_, c)| c.clone())
        .unwrap_or_default()
}

/// The name people know a kind of device by.
fn kind_name(kind: ffi::AVHWDeviceType) -> &'static str {
    use ffi::AVHWDeviceType::*;
    match kind {
        AV_HWDEVICE_TYPE_D3D11VA => "D3D11",
        AV_HWDEVICE_TYPE_DXVA2 => "DXVA2",
        AV_HWDEVICE_TYPE_CUDA => "CUDA",
        AV_HWDEVICE_TYPE_VAAPI => "VA-API",
        AV_HWDEVICE_TYPE_VIDEOTOOLBOX => "VideoToolbox",
        _ => "another kind",
    }
}

/// A device of `kind`; for VA-API on any card that opens, not only the default one.
fn open_kind(kind: ffi::AVHWDeviceType) -> Result<BufferRef, i32> {
    if kind == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI {
        open_vaapi()
    } else {
        open_device_of(kind)
    }
}

/// The first of [`device_kinds`] that opens, if any, and why each one before it did not.
///
/// FFmpeg's log is held quiet while this runs: a computer without a VA-API driver, or with a card
/// that cannot decode the codec, is the ordinary case, and FFmpeg reports every failed attempt as
/// an error on stderr that reads like the program is broken. Why each kind failed is still said,
/// at debug level, for whoever is looking into it.
///
/// The level is put back to `Error`, the one [`crate::init`] sets, rather than to the level found
/// here: the encoder check (`picasso_export::check_hardware`) quiets FFmpeg on a thread of its own
/// at start, and a level saved while it runs would be Quiet, leaving FFmpeg silent for the rest
/// of the run.
fn open_device() -> (Option<(ffi::AVHWDeviceType, BufferRef)>, Vec<String>) {
    let _probing = PROBING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    probe_devices()
}

/// Held while devices are probed: FFmpeg's log level is one for the whole program, so two probes
/// at once (the decoders' and [`check`]'s) would put it back under each other.
static PROBING: Mutex<()> = Mutex::new(());

/// [`open_device`]'s work, with [`PROBING`] held.
fn probe_devices() -> (Option<(ffi::AVHWDeviceType, BufferRef)>, Vec<String>) {
    ff::util::log::set_level(ff::util::log::Level::Quiet);
    let mut failed = Vec::new();
    let found = device_kinds()
        .iter()
        .find_map(|kind| match open_kind(*kind) {
            Ok(device) => Some((*kind, device)),
            Err(code) => {
                let why = format!("{}: {}", kind_name(*kind), ff::Error::from(code));
                tracing::debug!("no hardware decoding with {why}");
                failed.push(why);
                None
            }
        });
    ff::util::log::set_level(ff::util::log::Level::Error);
    (found, failed)
}

/// Whether video can be decoded on the graphics card here, for `picasso-suite --graphics-check`:
/// the kind of device that opens, or a line for each kind that did not (with what to look at on
/// Linux, where a missing VA-API driver is the usual reason). Opens a device of its own and lets
/// it go; the decoders' shared one is not touched.
pub fn check() -> Result<&'static str, Vec<String>> {
    crate::init();
    if !hardware_decoding() {
        return Err(vec![
            "turned off (System Preferences > Decode Options, or PICASSO_NO_HWACCEL)".into(),
        ]);
    }
    let (found, mut failed) = open_device();
    if let Some((kind, _device)) = found {
        return Ok(kind_name(kind));
    }
    if cfg!(target_os = "linux") {
        let render_node = std::fs::read_dir("/dev/dri").is_ok_and(|dir| {
            dir.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        });
        failed.push(if render_node {
            "VA-API opens the card's render node (/dev/dri/renderD*) but its driver did not start: \
             it is usually not installed (Intel: intel-media-va-driver, or i965-va-driver for \
             cards before 2014; AMD: mesa-va-drivers). `vainfo` says which driver it looks for."
                .into()
        } else {
            "there is no render node in /dev/dri: the graphics driver is not loaded, or this \
             sandbox cannot see it"
                .into()
        });
    }
    Err(failed)
}

/// The device the decoders share: opened the first time one asks, once for the whole run. `None`
/// inside means none of [`device_kinds`] opened, which is not tried again.
static DEVICE: Mutex<Option<Option<(ffi::AVHWDeviceType, BufferRef)>>> = Mutex::new(None);

/// The decoder's pixel format for pictures that stay on a device of `kind`, if it has one.
fn hardware_format(codec: &ff::Codec, kind: ffi::AVHWDeviceType) -> Option<ffi::AVPixelFormat> {
    let method = ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32;
    for i in 0..32 {
        // SAFETY: the codec is FFmpeg's; past the last configuration the answer is null.
        let config = unsafe { ffi::avcodec_get_hw_config(codec.as_ptr(), i) };
        if config.is_null() {
            return None;
        }
        // SAFETY: not null, and FFmpeg's configurations live as long as the program.
        let config = unsafe { &*config };
        if config.methods & method != 0 && config.device_type == kind {
            return Some(config.pix_fmt);
        }
    }
    None
}

/// Chooses the hardware format kept in `opaque` when the decoder offers it; otherwise FFmpeg's own
/// choice, which decodes on the processor.
unsafe extern "C" fn pick_format(
    ctx: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    // SAFETY: FFmpeg passes its context and a list that ends with AV_PIX_FMT_NONE.
    unsafe {
        let wanted = (*ctx).opaque as isize as i32;
        let mut p = formats;
        while !p.is_null() && *p != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            if *p as i32 == wanted {
                return *p;
            }
            p = p.add(1);
        }
        ffi::avcodec_default_get_format(ctx, formats)
    }
}

/// Sets up a decoder that is not open yet to decode on the graphics card, when hardware decoding
/// is on, the codec is one of [`CODECS`] and a device opens. Returns whether it did; the decoder
/// still falls back to the processor by itself if the card turns the video down.
pub(crate) fn attach(ctx: &mut ff::codec::context::Context) -> bool {
    if !hardware_decoding() || !CODECS.contains(&ctx.id()) {
        return false;
    }
    let Some(codec) = ff::decoder::find(ctx.id()) else {
        return false;
    };
    let Ok(mut shared) = DEVICE.lock() else {
        return false;
    };
    let device = shared.get_or_insert_with(|| {
        let (found, failed) = open_device();
        match &found {
            Some((kind, _)) => tracing::info!("hardware decoding with {}", kind_name(*kind)),
            None => tracing::info!(
                "no hardware decoder: decoding on the processor ({})",
                failed.join("; ")
            ),
        }
        found
    });
    let Some((kind, device)) = device.as_ref() else {
        return false;
    };
    let Some(format) = hardware_format(&codec, *kind) else {
        return false;
    };
    // SAFETY: the context is not open; it takes the new device reference and frees it with
    // itself, and `opaque` is otherwise unused (ffmpeg-next never sets it).
    unsafe {
        let raw = ctx.as_mut_ptr();
        (*raw).hw_device_ctx = device.new_ref();
        (*raw).opaque = format as i32 as isize as *mut c_void;
        (*raw).get_format = Some(pick_format);
    }
    true
}

/// Whether a decoded picture is still on the graphics card.
pub(crate) fn on_card(frame: &ff::frame::Video) -> bool {
    // SAFETY: reading a field of a frame we hold.
    unsafe { !(*frame.as_ptr()).hw_frames_ctx.is_null() }
}

/// Copies a picture from the graphics card into memory (NV12 or P010), with its time.
pub(crate) fn download(frame: &ff::frame::Video) -> Result<ff::frame::Video, ff::Error> {
    let mut out = ff::frame::Video::empty();
    // SAFETY: both frames are allocated; FFmpeg allocates `out`'s buffers.
    unsafe {
        let r = ffi::av_hwframe_transfer_data(out.as_mut_ptr(), frame.as_ptr(), 0);
        if r < 0 {
            return Err(ff::Error::from(r));
        }
        let r = ffi::av_frame_copy_props(out.as_mut_ptr(), frame.as_ptr());
        if r < 0 {
            return Err(ff::Error::from(r));
        }
    }
    Ok(out)
}

/// A pool of pictures on the graphics card for a VA-API encoder (Linux), which takes pictures only
/// from the card.
pub struct VaapiFrames {
    frames: BufferRef,
}

impl VaapiFrames {
    /// A pool of `width` x `height` pictures that hold `layout` (NV12, or P010 for 10 bits) on the
    /// card with render node `node`, or on the default card (else the first that opens) when
    /// there is none.
    pub fn new(width: u32, height: u32, layout: Pixel, node: Option<&str>) -> Result<Self, String> {
        let device = match node {
            Some(node) => {
                open_device_named(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, Some(node))
            }
            None => open_vaapi(),
        }
        .map_err(|_| "no VA-API device: is there a graphics driver with VA-API?".to_string())?;
        // SAFETY: the pool takes its own reference to the device; the context fields are set
        // before it is initialised, as FFmpeg asks.
        unsafe {
            let raw = ffi::av_hwframe_ctx_alloc(device.0);
            if raw.is_null() {
                return Err("could not make a VA-API picture pool".into());
            }
            let frames = BufferRef(raw);
            let ctx = (*raw).data as *mut ffi::AVHWFramesContext;
            (*ctx).format = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
            (*ctx).sw_format = layout.into();
            (*ctx).width = width as i32;
            (*ctx).height = height as i32;
            (*ctx).initial_pool_size = 20;
            let r = ffi::av_hwframe_ctx_init(raw);
            if r < 0 {
                return Err(format!(
                    "the VA-API picture pool did not start: {}",
                    ff::Error::from(r)
                ));
            }
            Ok(Self { frames })
        }
    }

    /// Gives an encoder that is not open yet the pool its pictures come from.
    pub fn attach(&self, encoder: &mut ff::codec::context::Context) {
        // SAFETY: the encoder is not open and takes its own reference.
        unsafe { (*encoder.as_mut_ptr()).hw_frames_ctx = self.frames.new_ref() }
    }

    /// Copies a picture in memory to the card, with its time.
    pub fn upload(&self, picture: &ff::frame::Video) -> Result<ff::frame::Video, ff::Error> {
        let mut out = ff::frame::Video::empty();
        // SAFETY: `out` is allocated and takes a picture from the pool; `picture` is in the
        // pool's layout and size.
        unsafe {
            let r = ffi::av_hwframe_get_buffer(self.frames.0, out.as_mut_ptr(), 0);
            if r < 0 {
                return Err(ff::Error::from(r));
            }
            let r = ffi::av_hwframe_transfer_data(out.as_mut_ptr(), picture.as_ptr(), 0);
            if r < 0 {
                return Err(ff::Error::from(r));
            }
            let r = ffi::av_frame_copy_props(out.as_mut_ptr(), picture.as_ptr());
            if r < 0 {
                return Err(ff::Error::from(r));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remembered_choice_is_found_by_encoder_and_replaced() {
        assert_eq!(vaapi_choice("never_checked_vaapi"), VaapiChoice::default());
        let a = VaapiChoice {
            node: Some("/dev/dri/renderD129".into()),
            low_power: true,
        };
        remember_vaapi_choice("test_encoder_vaapi", a.clone());
        assert_eq!(vaapi_choice("test_encoder_vaapi"), a);
        remember_vaapi_choice("test_encoder_vaapi", VaapiChoice::default());
        assert_eq!(vaapi_choice("test_encoder_vaapi"), VaapiChoice::default());
    }

    #[test]
    fn render_nodes_are_listed_in_order() {
        let nodes = vaapi_nodes();
        assert!(nodes.windows(2).all(|w| w[0] < w[1]));
        assert!(nodes.iter().all(|n| n.contains("renderD")));
    }

    #[test]
    fn a_missing_render_node_is_an_error_not_a_crash() {
        let r = open_device_named(
            ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
            Some("/dev/dri/renderD999"),
        );
        assert!(r.is_err());
        let r = open_device_named(
            ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
            Some("bad\0name"),
        );
        assert!(r.is_err());
    }

    #[test]
    fn the_switch_and_the_environment_decide() {
        set_hardware_decoding(false);
        assert!(!hardware_decoding());
        set_hardware_decoding(true);
        assert_eq!(
            hardware_decoding(),
            std::env::var_os("PICASSO_NO_HWACCEL").is_none()
        );
    }

    #[test]
    fn only_long_gop_codecs_use_the_card() {
        crate::init();
        let Some(codec) = ff::decoder::find(ff::codec::Id::PRORES) else {
            return;
        };
        let mut ctx = ff::codec::context::Context::new_with_codec(codec);
        assert!(!attach(&mut ctx), "ProRes stays on the processor");
    }

    #[test]
    fn the_probe_leaves_the_log_at_error_even_if_it_starts_quiet() {
        crate::init();
        // The other tests probe too, so the probes are kept out while this one looks.
        let _probing = PROBING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The encoder check may have FFmpeg quiet when the probe starts; the probe must not keep
        // it that way.
        ff::util::log::set_level(ff::util::log::Level::Quiet);
        let _ = probe_devices();
        assert_eq!(
            ff::util::log::get_level().ok(),
            Some(ff::util::log::Level::Error),
            "the probe must not leave FFmpeg quiet"
        );
    }

    #[test]
    fn the_check_names_the_device_or_says_why_there_is_none() {
        match check() {
            Ok(kind) => assert!(!kind.is_empty()),
            Err(why) => assert!(!why.is_empty() && why.iter().all(|w| !w.is_empty())),
        }
    }

    #[test]
    fn cuda_is_only_asked_for_when_nvidia_drives_the_card() {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use ffi::AVHWDeviceType::*;
            let kinds = device_kinds();
            assert_eq!(kinds[0], AV_HWDEVICE_TYPE_VAAPI);
            assert_eq!(kinds.contains(&AV_HWDEVICE_TYPE_CUDA), nvidia_driver());
        }
    }
}
