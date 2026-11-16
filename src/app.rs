use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

#[cfg(not(target_os = "android"))]
use crate::scanner::scan_folder;
use crate::scanner::ScannedFile;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

pub fn run() -> Result<(), slint::PlatformError> {
    let app = App::new()?;
    let records = Arc::new(Mutex::new(Vec::<ScannedFile>::new()));

    #[cfg(target_os = "android")]
    app.set_android_platform(true);
    #[cfg(target_os = "android")]
    crate::android::set_ui_handle(app.as_weak());
    #[cfg(target_os = "android")]
    match crate::android::restore_saved_folder() {
        Ok(Some(folder_name)) => app.set_root_path(folder_name.into()),
        Ok(None) => {}
        Err(error) => app.set_status(error.into()),
    }

    #[cfg(not(target_os = "android"))]
    {
        let weak = app.as_weak();
        app.on_choose_folder(move || {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                if let Some(app) = weak.upgrade() {
                    app.set_root_path(path.display().to_string().into());
                }
            }
        });
    }

    #[cfg(target_os = "android")]
    app.on_choose_folder({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                match crate::android::open_folder_picker() {
                    Ok(()) => app.set_status("Откройте системный выбор папки.".into()),
                    Err(error) => app.set_status(error.into()),
                }
            }
        }
    });

    app.on_scan_folder({
        let weak = app.as_weak();
        let records = records.clone();
        move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let root = app.get_root_path().to_string();
            if root.is_empty() {
                app.set_status("Сначала выберите папку или укажите путь.".into());
                return;
            }

            app.set_status("Сканирование папки и проверка содержимого файлов…".into());
            let weak = weak.clone();
            let records = records.clone();
            thread::spawn(move || {
                let result = scan_selected_folder(&root);
                if let Err(error) = slint::invoke_from_event_loop(move || {
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    match result {
                        Ok(result) => {
                            let count = result.files.len();
                            let duplicates = result.files.iter().filter(|file| file.duplicate).count();
                            let installers = result
                                .files
                                .iter()
                                .filter(|file| file.large_installer)
                                .count();
                            let error_count = result.errors.len();
                            *records.lock().expect("file list lock poisoned") = result.files;
                            refresh_files(&app, &records);
                            let mut status = format!(
                                "Найдено файлов: {count} · копий: {duplicates} · крупных установщиков: {installers}"
                            );
                            if error_count > 0 {
                                status.push_str(&format!(
                                    " · ошибки сканирования (до {error_count} примеров): {}",
                                    result.errors.join(" | ")
                                ));
                            }
                            app.set_status(status.into());
                        }
                        Err(error) => app.set_status(error.into()),
                    }
                }) {
                    eprintln!("Не удалось обновить интерфейс после сканирования: {error}");
                }
            });
        }
    });

    app.on_selection_changed({
        let weak = app.as_weak();
        let records = records.clone();
        move |id, selected| {
            if let Some(file) = records
                .lock()
                .expect("file list lock poisoned")
                .get_mut(id as usize)
            {
                file.selected = selected;
            }
            if let Some(app) = weak.upgrade() {
                refresh_files(&app, &records);
            }
        }
    });

    app.on_select_all({
        let weak = app.as_weak();
        let records = records.clone();
        move || {
            for file in records.lock().expect("file list lock poisoned").iter_mut() {
                file.selected = true;
            }
            if let Some(app) = weak.upgrade() {
                refresh_files(&app, &records);
            }
        }
    });

    app.on_request_delete({
        let weak = app.as_weak();
        let records = records.clone();
        move || {
            let selected = records
                .lock()
                .expect("file list lock poisoned")
                .iter()
                .filter(|file| file.selected)
                .count();
            if let Some(app) = weak.upgrade() {
                if selected == 0 {
                    app.set_status("Сначала отметьте файлы для удаления.".into());
                } else {
                    app.set_confirm_delete(true);
                }
            }
        }
    });

    app.on_confirm_delete_files({
        let weak = app.as_weak();
        let records = records.clone();
        move || {
            let selected: Vec<PathBuf> = records
                .lock()
                .expect("file list lock poisoned")
                .iter()
                .filter(|file| file.selected)
                .map(|file| file.path.clone())
                .collect();
            if let Some(app) = weak.upgrade() {
                app.set_confirm_delete(false);
                app.set_status(format!("Удаление файлов: {}…", selected.len()).into());
            }

            let weak = weak.clone();
            let records = records.clone();
            thread::spawn(move || {
                let mut deleted = Vec::new();
                let mut errors = Vec::new();
                for path in selected {
                    match delete_selected_file(&path) {
                        Ok(()) => deleted.push(path),
                        Err(error) => {
                            errors.push(format!("{}: {error}", path.display()));
                        }
                    }
                }

                if let Err(error) = slint::invoke_from_event_loop(move || {
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    let deleted_count = deleted.len();
                    let deleted_paths: std::collections::HashSet<PathBuf> =
                        deleted.into_iter().collect();
                    records
                        .lock()
                        .expect("file list lock poisoned")
                        .retain(|file| !deleted_paths.contains(&file.path));
                    refresh_files(&app, &records);
                    let mut status = format!("Удалено файлов: {deleted_count}");
                    if !errors.is_empty() {
                        status.push_str(&format!(
                            " · ошибок: {} · {}",
                            errors.len(),
                            errors
                                .iter()
                                .take(5)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(" | ")
                        ));
                    }
                    app.set_status(status.into());
                }) {
                    eprintln!("Не удалось обновить интерфейс после удаления: {error}");
                }
            });
        }
    });

    let weak = app.as_weak();
    let records_for_sort = records.clone();
    app.on_toggle_sort_by_extension(move || {
        if let Some(app) = weak.upgrade() {
            app.set_sort_by_extension(!app.get_sort_by_extension());
            refresh_files(&app, &records_for_sort);
        }
    });

    let weak = app.as_weak();
    let records_for_search = records.clone();
    app.on_search_changed(move |_| {
        if let Some(app) = weak.upgrade() {
            refresh_files(&app, &records_for_search);
        }
    });

    let weak = app.as_weak();
    let records_for_extension = records.clone();
    app.on_extension_filter_changed(move |_| {
        if let Some(app) = weak.upgrade() {
            refresh_files(&app, &records_for_extension);
        }
    });

    app.run()
}

fn refresh_files(app: &App, records: &Arc<Mutex<Vec<ScannedFile>>>) {
    let search = app.get_search().to_string().to_lowercase();
    let extension_filter = app.get_extension_filter().to_string().trim().to_lowercase();
    let extension_filter = if extension_filter.is_empty() || extension_filter == "." {
        String::new()
    } else if extension_filter.starts_with('.') {
        extension_filter
    } else {
        format!(".{extension_filter}")
    };
    let sort_by_extension = app.get_sort_by_extension();

    let records = records.lock().expect("file list lock poisoned");
    let mut visible: Vec<(usize, &ScannedFile)> = records
        .iter()
        .enumerate()
        .filter(|(_, file)| {
            (search.is_empty()
                || file.name.to_lowercase().contains(&search)
                || file.display_path.to_lowercase().contains(&search))
                && (extension_filter.is_empty() || file.extension == extension_filter)
        })
        .collect();

    if sort_by_extension {
        visible.sort_by(|(_, left), (_, right)| {
            left.extension
                .cmp(&right.extension)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });
    } else {
        visible.sort_by(|(_, left), (_, right)| {
            left.name.to_lowercase().cmp(&right.name.to_lowercase())
        });
    }

    let items = visible
        .into_iter()
        .map(|(id, file)| {
            let mut tags = Vec::new();
            if file.duplicate {
                tags.push("Дубликат");
            }
            if file.large_installer {
                tags.push("Установщик >100 МБ");
            }
            FileItem {
                id: id as i32,
                name: SharedString::from(file.name.as_str()),
                path: SharedString::from(file.display_path.as_str()),
                extension: SharedString::from(file.extension.as_str()),
                size: SharedString::from(format_size(file.size)),
                tags: SharedString::from(tags.join(" · ")),
                selected: file.selected,
            }
        })
        .collect::<Vec<_>>();
    app.set_files(ModelRc::new(VecModel::from(items)));
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["Б", "КБ", "МБ", "ГБ"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

#[cfg(not(target_os = "android"))]
fn delete_selected_file(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            trash::delete(path).map_err(|error| error.to_string())
        }
        Ok(_) => Err("объект изменился или больше не является обычным файлом".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(target_os = "android")]
fn scan_selected_folder(_display_path: &str) -> Result<crate::scanner::ScanResult, String> {
    crate::android::scan_selected_folder()
}

#[cfg(not(target_os = "android"))]
fn scan_selected_folder(display_path: &str) -> Result<crate::scanner::ScanResult, String> {
    scan_folder(Path::new(display_path))
}

#[cfg(target_os = "android")]
fn delete_selected_file(path: &Path) -> Result<(), String> {
    crate::android::delete_document(path.to_string_lossy().as_ref())
}
