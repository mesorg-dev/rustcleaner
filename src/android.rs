#[cfg(any(target_os = "android", test))]
use std::sync::{Mutex, OnceLock};

#[cfg(any(target_os = "android", test))]
use jni::JavaVM;
#[cfg(any(target_os = "android", test))]
use jni::{objects::JClass, EnvUnowned};
#[cfg(any(target_os = "android", test))]
use jni::{
    objects::{JObject, JString, JValue},
    signature::RuntimeMethodSignature,
    strings::JNIString,
    Env,
};
#[cfg(target_os = "android")]
use serde::Deserialize;
#[cfg(any(target_os = "android", test))]
use slint::Weak;

#[cfg(any(target_os = "android", test))]
use crate::app::App;
#[cfg(target_os = "android")]
use crate::scanner::ScanResult;
#[cfg(target_os = "android")]
use crate::scanner::{scan_remote_files, RemoteFile};

#[cfg(any(target_os = "android", test))]
static UI_HANDLE: OnceLock<Mutex<Option<Weak<App>>>> = OnceLock::new();
#[cfg(any(target_os = "android", test))]
static SELECTED_TREE_URI: OnceLock<Mutex<Option<String>>> = OnceLock::new();

#[cfg(target_os = "android")]
static ANDROID_APP: OnceLock<Mutex<Option<slint::android::AndroidApp>>> = OnceLock::new();

#[cfg(target_os = "android")]
#[derive(Deserialize)]
struct SavedFolder {
    uri: String,
    display_name: String,
}

#[cfg(target_os = "android")]
#[derive(Deserialize)]
struct SafScanResult {
    files: Vec<SafFile>,
    errors: Vec<String>,
}

#[cfg(target_os = "android")]
#[derive(Deserialize)]
struct SafFile {
    uri: String,
    display_path: String,
    name: String,
    size: u64,
}

#[cfg(target_os = "android")]
pub(crate) fn set_ui_handle(handle: Weak<App>) {
    *UI_HANDLE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
}

#[cfg(target_os = "android")]
pub(crate) fn set_android_app(app: slint::android::AndroidApp) {
    *ANDROID_APP
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(app);
}

#[cfg(target_os = "android")]
pub(crate) fn open_folder_picker() -> Result<(), String> {
    let app = android_app()?;
    let java_thread_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = with_activity(&java_thread_app, |env, activity| {
            call_void_method_without_argument(env, activity, "openFolderPicker")
        }) {
            eprintln!("Не удалось открыть Android-пикер папок: {error}");
        }
    }));
    Ok(())
}

#[cfg(target_os = "android")]
pub(crate) fn restore_saved_folder() -> Result<Option<String>, String> {
    let app = android_app()?;
    let saved = with_activity(&app, |env, activity| {
        call_string_method_without_argument(env, activity, "restoreSavedFolder")
    })?;
    if saved.is_empty() {
        return Ok(None);
    }

    let folder: SavedFolder = serde_json::from_str(&saved)
        .map_err(|error| format!("Не удалось прочитать сохранённую папку Android: {error}"))?;
    *SELECTED_TREE_URI
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(folder.uri);
    Ok(Some(folder.display_name))
}

#[cfg(target_os = "android")]
pub(crate) fn scan_selected_folder() -> Result<ScanResult, String> {
    let uri = SELECTED_TREE_URI
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or_else(|| "Выберите папку через системный пикер Android.".to_owned())?;
    let app = android_app()?;

    with_activity(&app, |env, activity| {
        let response = call_string_method(env, activity, "scanTree", &uri)?;
        let response: SafScanResult = serde_json::from_str(&response)
            .map_err(|error| format!("Не удалось разобрать результат SAF-сканирования: {error}"))?;
        let remote_files = response
            .files
            .into_iter()
            .map(|file| RemoteFile {
                uri: file.uri,
                display_path: file.display_path,
                name: file.name,
                size: file.size,
            })
            .collect();
        scan_remote_files(remote_files, response.errors, |document_uri| {
            let result = call_string_method(env, activity, "hashDocument", document_uri)?;
            match result.strip_prefix("!error:") {
                Some(error) => Err(error.to_owned()),
                None => Ok(result),
            }
        })
    })
}

#[cfg(target_os = "android")]
pub(crate) fn delete_document(uri: &str) -> Result<(), String> {
    let app = android_app()?;
    let result = with_activity(&app, |env, activity| {
        call_string_method(env, activity, "deleteDocument", uri)
    })?;
    if result.is_empty() {
        Ok(())
    } else {
        Err(result)
    }
}

#[cfg(any(target_os = "android", test))]
#[allow(dead_code)]
fn call_string_method(
    env: &mut Env<'_>,
    activity: &JObject<'_>,
    method: &str,
    argument: &str,
) -> Result<String, String> {
    let argument = env
        .new_string(argument)
        .map_err(|error| error.to_string())?;
    let method_name = JNIString::new(method);
    let signature = RuntimeMethodSignature::from_str("(Ljava/lang/String;)Ljava/lang/String;")
        .map_err(|error| error.to_string())?;
    let result = env
        .call_method(
            activity,
            &method_name,
            signature.method_signature(),
            &[JValue::Object(&argument)],
        )
        .and_then(|value| value.l())
        .map_err(|error| error.to_string())?;
    if result.is_null() {
        return Ok(String::new());
    }

    env.as_cast::<JString>(&result)
        .map_err(|error| error.to_string())?
        .try_to_string(env)
        .map_err(|error| error.to_string())
}

#[cfg(any(target_os = "android", test))]
#[allow(dead_code)]
fn call_string_method_without_argument(
    env: &mut Env<'_>,
    activity: &JObject<'_>,
    method: &str,
) -> Result<String, String> {
    let method_name = JNIString::new(method);
    let signature = RuntimeMethodSignature::from_str("()Ljava/lang/String;")
        .map_err(|error| error.to_string())?;
    let result = env
        .call_method(activity, &method_name, signature.method_signature(), &[])
        .and_then(|value| value.l())
        .map_err(|error| error.to_string())?;
    if result.is_null() {
        return Ok(String::new());
    }

    env.as_cast::<JString>(&result)
        .map_err(|error| error.to_string())?
        .try_to_string(env)
        .map_err(|error| error.to_string())
}

#[cfg(any(target_os = "android", test))]
#[allow(dead_code)]
fn call_void_method_without_argument(
    env: &mut Env<'_>,
    activity: &JObject<'_>,
    method: &str,
) -> Result<(), String> {
    let method_name = JNIString::new(method);
    let signature = RuntimeMethodSignature::from_str("()V").map_err(|error| error.to_string())?;
    env.call_method(activity, &method_name, signature.method_signature(), &[])
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "android")]
fn android_app() -> Result<slint::android::AndroidApp, String> {
    ANDROID_APP
        .get()
        .and_then(|app| {
            app.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        })
        .ok_or_else(|| "Android-приложение ещё не инициализировано.".to_owned())
}

#[cfg(target_os = "android")]
fn with_activity<T>(
    app: &slint::android::AndroidApp,
    callback: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> Result<T, String>,
) -> Result<T, String> {
    with_vm_activity(app.vm_as_ptr(), app.activity_as_ptr(), callback)
}

#[cfg(any(target_os = "android", test))]
#[allow(dead_code)]
fn with_vm_activity<T>(
    vm_pointer: *mut std::ffi::c_void,
    activity_pointer: *mut std::ffi::c_void,
    callback: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let vm = unsafe { JavaVM::from_raw(vm_pointer.cast()) };
    vm.attach_current_thread(|env| -> jni::errors::Result<Result<T, String>> {
        let activity = unsafe { JObject::from_raw(env, activity_pointer.cast()) };
        Ok(callback(env, &activity))
    })
    .map_err(|error| format!("Ошибка подключения к Android API: {error}"))?
}

#[cfg(any(target_os = "android", test))]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_mesorg_rustcleaner_RustCleanerActivity_nativeFolderResult(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    uri: JString<'_>,
    display_name: JString<'_>,
    warning: JString<'_>,
) {
    let outcome = env.with_env(|env| -> jni::errors::Result<()> {
        let uri = read_optional_string(env, &uri)?;
        let display_name = read_optional_string(env, &display_name)?;
        let warning = read_optional_string(env, &warning)?;

        if let Some(uri) = uri {
            *SELECTED_TREE_URI
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(uri);
        }

        let weak = UI_HANDLE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(weak) = weak else {
            eprintln!("Не удалось обновить интерфейс после выбора Android-папки.");
            return Ok(());
        };

        if let Err(error) = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                match (display_name, warning) {
                    (Some(name), Some(warning)) if !warning.is_empty() => {
                        app.set_root_path(name.into());
                        app.set_status(warning.into());
                    }
                    (Some(name), _) => {
                        app.set_root_path(name.into());
                        app.set_status(
                            "Папка выбрана. Доступ ограничен только её содержимым.".into(),
                        );
                    }
                    (None, Some(error)) => app.set_status(error.into()),
                    (None, None) => app.set_status("Выбор папки отменён.".into()),
                }
            }
        }) {
            eprintln!("Не удалось обновить интерфейс после выбора Android-папки: {error}");
        }
        Ok(())
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

#[cfg(any(target_os = "android", test))]
fn read_optional_string(
    env: &mut Env<'_>,
    value: &JString<'_>,
) -> jni::errors::Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    value.try_to_string(env).map(Some)
}
