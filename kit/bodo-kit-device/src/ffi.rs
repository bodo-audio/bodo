//! The only unsafe code in this crate: on macOS, finding an audio device's IO workgroup and joining it.
//!
//! CoreAudio runs each device's IO thread in an `os_workgroup`. Threads that help it, like the RT workers, join that
//! workgroup so the scheduler knows they serve the same deadline, and on Apple Silicon keeps them on performance
//! cores.

use std::ffi::{c_int, c_void};
use std::ptr;
use std::sync::Arc;

use bodo_kit_core::pool::{Membership, ThreadGroup};

type AudioObjectId = u32;
type OsStatus = i32;

/// `AudioObjectPropertyAddress`.
#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

/// `os_workgroup_join_token_s`: a signature word and 36 opaque bytes.
#[repr(C)]
struct JoinToken {
    sig: u32,
    opaque: [u8; 36],
}

const SYSTEM_OBJECT: AudioObjectId = 1;
const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
const ELEMENT_MAIN: u32 = 0;
const TRANSLATE_UID_TO_DEVICE: u32 = u32::from_be_bytes(*b"uidd");
const IO_THREAD_OS_WORKGROUP: u32 = u32::from_be_bytes(*b"oswg");
const UTF8: u32 = 0x0800_0100;

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyData(
        object: AudioObjectId,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        data_size: *mut u32,
        data: *mut c_void,
    ) -> OsStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithBytes(allocator: *const c_void, bytes: *const u8, len: isize, encoding: u32, external: u8) -> *const c_void;
    fn CFRelease(object: *const c_void);
}

// In libSystem, which every macOS program links.
unsafe extern "C" {
    fn os_workgroup_join(workgroup: *mut c_void, token: *mut JoinToken) -> c_int;
    fn os_workgroup_leave(workgroup: *mut c_void, token: *mut JoinToken);
    fn os_retain(object: *mut c_void) -> *mut c_void;
    fn os_release(object: *mut c_void);
}

/// One reference to an `os_workgroup_t`, released on drop.
struct Workgroup(*mut c_void);

// SAFETY: `os_workgroup_t` is a reference-counted OS object whose functions may be called from any thread; joining
// it from the threads that should belong to it is its purpose. The pointer is only used through those functions.
unsafe impl Send for Workgroup {}
// SAFETY: as above; `join` takes `&self` and the OS synchronizes membership internally.
unsafe impl Sync for Workgroup {}

impl Drop for Workgroup {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a non-null workgroup this value holds one reference to, and this releases exactly that.
        unsafe { os_release(self.0) }
    }
}

impl ThreadGroup for Workgroup {
    fn join(&self) -> Option<Membership> {
        // Boxed so the token keeps its address from join to leave.
        let mut token = Box::new(JoinToken { sig: 0, opaque: [0; 36] });
        // SAFETY: `self.0` is a live workgroup, and `token` points to writable memory of the type the call expects.
        let result = unsafe { os_workgroup_join(self.0, &raw mut *token) };
        if result != 0 {
            tracing::debug!(result, "could not join the audio device's workgroup");
            return None;
        }
        // SAFETY: `self.0` is live; the new reference keeps the workgroup alive until this thread has left it.
        let workgroup = Workgroup(unsafe { os_retain(self.0) });
        Some(Membership::new(move || {
            // SAFETY: `workgroup` is the group this thread joined with `token`, and `Membership` runs this on the
            // joining thread, once.
            unsafe { os_workgroup_leave(workgroup.0, &raw mut *token) };
        }))
    }
}

/// The IO workgroup of the CoreAudio device with this UID, or `None` if there is no such device or it has none.
///
/// Talks to CoreAudio.
pub(crate) fn device_workgroup(uid: &str) -> Option<Arc<dyn ThreadGroup>> {
    let device = device_for_uid(uid)?;
    let address = PropertyAddress { selector: IO_THREAD_OS_WORKGROUP, scope: SCOPE_GLOBAL, element: ELEMENT_MAIN };
    let mut workgroup: *mut c_void = ptr::null_mut();
    let mut size = size_of::<*mut c_void>() as u32;
    // SAFETY: the address is valid, there is no qualifier, and the output is a pointer-sized `os_workgroup_t` slot of
    // the stated size. CoreAudio returns the workgroup retained, and `Workgroup` takes over that reference.
    let status = unsafe { AudioObjectGetPropertyData(device, &raw const address, 0, ptr::null(), &raw mut size, (&raw mut workgroup).cast()) };
    if status != 0 || workgroup.is_null() {
        tracing::debug!(status, uid, "the audio device has no IO workgroup");
        return None;
    }
    Some(Arc::new(Workgroup(workgroup)))
}

/// The CoreAudio device id for a device UID.
fn device_for_uid(uid: &str) -> Option<AudioObjectId> {
    let len = isize::try_from(uid.len()).ok()?;
    // SAFETY: `uid` is valid UTF-8 for `len` bytes; the call copies it and returns a new string or null.
    let string = unsafe { CFStringCreateWithBytes(ptr::null(), uid.as_ptr(), len, UTF8, 0) };
    if string.is_null() {
        return None;
    }
    let address = PropertyAddress { selector: TRANSLATE_UID_TO_DEVICE, scope: SCOPE_GLOBAL, element: ELEMENT_MAIN };
    let mut device: AudioObjectId = 0;
    let mut size = size_of::<AudioObjectId>() as u32;
    // SAFETY: the qualifier is the `CFStringRef` itself, passed by address with its size, as the property documents;
    // the output is an `AudioObjectID` slot of the stated size.
    let status = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &raw const address,
            size_of::<*const c_void>() as u32,
            (&raw const string).cast(),
            &raw mut size,
            (&raw mut device).cast(),
        )
    };
    // SAFETY: `string` came from a create function, so this code owns one reference and releases it once.
    unsafe { CFRelease(string) };
    (status == 0 && device != 0).then_some(device)
}
