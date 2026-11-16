mod android;
pub(crate) mod app;
pub(crate) mod scanner;

pub use app::run;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: slint::android::AndroidApp) {
    if let Err(error) = slint::android::init(app.clone()) {
        eprintln!("Не удалось инициализировать Android-интерфейс: {error}");
        return;
    }

    android::set_android_app(app);
    if let Err(error) = run() {
        eprintln!("Ошибка запуска RustCleaner: {error}");
    }
}
