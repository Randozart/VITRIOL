use crate::{Config, Store};
use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Mutex;
use vitriol_core::cache_lifetime::Pin;

pub struct Handle {
    store: Mutex<Store>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    path: String,
    config: Config,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Begin {
        session: String,
        branch: String,
    },
    Finish {
        generation: u64,
        success: bool,
    },
    ReleaseTag {
        generation: u64,
    },
    Prepare {
        generation: u64,
        model: String,
        layout: String,
        bytes: u64,
    },
    Commit {
        ticket: u64,
    },
    Abort {
        ticket: u64,
    },
    AcquireRead {
        id: String,
        generation: Option<u64>,
    },
    ReleaseRead {
        lease: u64,
    },
    Pin {
        id: String,
        pin: Pin,
        enabled: bool,
    },
    Collect {
        epoch: u64,
        dry_run: bool,
    },
    Audit,
    List,
}

fn dispatch(store: &mut Store, command: Command) -> Result<Value, String> {
    match command {
        Command::Begin { session, branch } => Ok(json!(store.begin(&session, &branch)?)),
        Command::Finish {
            generation,
            success,
        } => {
            store.finish(generation, success)?;
            Ok(json!(true))
        }
        Command::ReleaseTag { generation } => {
            store.release_tag(generation)?;
            Ok(json!(true))
        }
        Command::Prepare {
            generation,
            model,
            layout,
            bytes,
        } => Ok(json!(store.prepare(generation, &model, &layout, bytes)?)),
        Command::Commit { ticket } => Ok(json!(store.commit(ticket)?)),
        Command::Abort { ticket } => {
            store.abort(ticket)?;
            Ok(json!(true))
        }
        Command::AcquireRead { id, generation } => Ok(json!(store.acquire_read(&id, generation)?)),
        Command::ReleaseRead { lease } => {
            store.release_read(lease)?;
            Ok(json!(true))
        }
        Command::Pin { id, pin, enabled } => {
            store.pin(&id, pin, enabled)?;
            Ok(json!(true))
        }
        Command::Collect { epoch, dry_run } => store.collect(epoch, dry_run),
        Command::Audit => Ok(json!(store.audit()?)),
        Command::List => store.list(),
    }
}

fn reply(value: Value) -> *mut c_char {
    CString::new(value.to_string())
        .expect("JSON escapes NUL")
        .into_raw()
}

/// # Safety
/// Input must be a valid NUL-terminated UTF-8 string. If non-null, error_out
/// must be writable; its returned string must be freed by vitriol_cache_string_free.
#[no_mangle]
pub unsafe extern "C" fn vitriol_cache_open(
    input: *const c_char,
    error_out: *mut *mut c_char,
) -> *mut Handle {
    if !error_out.is_null() {
        unsafe {
            *error_out = std::ptr::null_mut();
        }
    }
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<Store, String> {
        if input.is_null() {
            return Err("null cache configuration".into());
        }
        let text = unsafe { CStr::from_ptr(input) }
            .to_str()
            .map_err(|e| e.to_string())?;
        let open: Open = serde_json::from_str(text).map_err(|e| e.to_string())?;
        Store::open(Path::new(&open.path), open.config)
    }))
    .unwrap_or_else(|_| Err("managed store panicked during open".into()));
    match result {
        Ok(store) => Box::into_raw(Box::new(Handle {
            store: Mutex::new(store),
        })),
        Err(error) => {
            if !error_out.is_null() {
                unsafe {
                    *error_out = reply(json!({"error":error}));
                }
            }
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// Handle must remain live for the call. Input must be a NUL-terminated UTF-8
/// command. Free the returned JSON with vitriol_cache_string_free.
#[no_mangle]
pub unsafe extern "C" fn vitriol_cache_call(
    handle: *mut Handle,
    input: *const c_char,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<Value, String> {
        if handle.is_null() || input.is_null() {
            return Err("null cache handle or command".into());
        }
        let text = unsafe { CStr::from_ptr(input) }
            .to_str()
            .map_err(|e| e.to_string())?;
        let command = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let handle = unsafe { &*handle };
        let mut store = handle
            .store
            .lock()
            .map_err(|_| "managed store lock poisoned")?;
        dispatch(&mut store, command)
    }))
    .unwrap_or_else(|_| Err("managed store panicked; lock may be poisoned".into()));
    match result {
        Ok(value) => reply(json!({"ok":true,"result":value})),
        Err(error) => reply(json!({"ok":false,"error":error})),
    }
}

/// # Safety
/// The handle must come from vitriol_cache_open and have no remaining users.
#[no_mangle]
pub unsafe extern "C" fn vitriol_cache_close(handle: *mut Handle) {
    if !handle.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(handle) })));
    }
}

/// # Safety
/// String must be null or a pointer returned by this library and not freed before.
#[no_mangle]
pub unsafe extern "C" fn vitriol_cache_string_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}
