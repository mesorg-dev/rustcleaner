package dev.mesorg.rustcleaner;

import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.DocumentsContract;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.InputStream;
import java.security.MessageDigest;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;

public final class RustCleanerActivity extends NativeActivity {
    private static final String TAG = "RustCleaner";
    private static final int PICK_FOLDER_REQUEST = 7104;
    private static final String PREFERENCES = "rustcleaner";
    private static final String TREE_URI = "tree_uri";
    private static final String TREE_NAME = "tree_name";

    private static native void nativeFolderResult(String uri, String displayName, String warning);

    public void openFolderPicker() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        intent.addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION
                        | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                        | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
                        | Intent.FLAG_GRANT_PREFIX_URI_PERMISSION);

        String savedUri = getPreferences(MODE_PRIVATE).getString(TREE_URI, null);
        if (savedUri != null) {
            intent.putExtra(DocumentsContract.EXTRA_INITIAL_URI, Uri.parse(savedUri));
        }

        try {
            startActivityForResult(intent, PICK_FOLDER_REQUEST);
        } catch (ActivityNotFoundException error) {
            nativeFolderResult(null, null, "На устройстве не найден системный выбор папки.");
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (requestCode != PICK_FOLDER_REQUEST) {
            super.onActivityResult(requestCode, resultCode, data);
            return;
        }
        if (resultCode != RESULT_OK || data == null || data.getData() == null) {
            nativeFolderResult(null, null, null);
            return;
        }

        Uri treeUri = data.getData();
        int grantFlags = data.getFlags()
                & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        try {
            getContentResolver().takePersistableUriPermission(treeUri, grantFlags);
        } catch (SecurityException error) {
            nativeFolderResult(null, null, "Android не сохранил разрешение на выбранную папку.");
            return;
        }

        String name = queryDisplayName(treeUri);
        getPreferences(MODE_PRIVATE)
                .edit()
                .putString(TREE_URI, treeUri.toString())
                .putString(TREE_NAME, name)
                .apply();

        String warning = (grantFlags & Intent.FLAG_GRANT_WRITE_URI_PERMISSION) == 0
                ? "Папка выбрана только для чтения; удаление недоступно."
                : null;
        nativeFolderResult(treeUri.toString(), name, warning);
    }

    public String restoreSavedFolder() {
        String uri = getPreferences(MODE_PRIVATE).getString(TREE_URI, null);
        if (uri == null) {
            return "";
        }

        JSONObject folder = new JSONObject();
        try {
            folder.put("uri", uri);
            folder.put(
                    "display_name",
                    getPreferences(MODE_PRIVATE).getString(TREE_NAME, "Выбранная папка"));
            return folder.toString();
        } catch (JSONException error) {
            throw new IllegalStateException("Не удалось восстановить сохранённую папку.", error);
        }
    }

    public String scanTree(String treeUriString) {
        JSONArray files = new JSONArray();
        JSONArray errors = new JSONArray();
        Uri treeUri = Uri.parse(treeUriString);
        String rootId;
        try {
            rootId = DocumentsContract.getTreeDocumentId(treeUri);
        } catch (RuntimeException error) {
            return scanResult(files, errors, error.getMessage());
        }

        Set<String> visitedDirectories = new HashSet<>();
        collectDocuments(treeUri, rootId, "", files, errors, visitedDirectories);
        JSONObject result = new JSONObject();
        try {
            result.put("files", files);
            result.put("errors", errors);
            return result.toString();
        } catch (JSONException error) {
            return scanResult(new JSONArray(), errors, error.getMessage());
        }
    }

    private void collectDocuments(
            Uri treeUri,
            String directoryId,
            String relativePath,
            JSONArray files,
            JSONArray errors,
            Set<String> visitedDirectories) {
        if (!visitedDirectories.add(directoryId)) {
            return;
        }

        Uri childrenUri =
                DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, directoryId);
        String[] columns = {
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE
        };

        try (Cursor cursor = getContentResolver().query(childrenUri, columns, null, null, null)) {
            if (cursor == null) {
                addError(errors, relativePath, "Хранилище не вернуло список файлов.");
                return;
            }

            int idColumn = cursor.getColumnIndexOrThrow(
                    DocumentsContract.Document.COLUMN_DOCUMENT_ID);
            int nameColumn = cursor.getColumnIndexOrThrow(
                    DocumentsContract.Document.COLUMN_DISPLAY_NAME);
            int mimeColumn = cursor.getColumnIndexOrThrow(
                    DocumentsContract.Document.COLUMN_MIME_TYPE);
            int sizeColumn = cursor.getColumnIndex(
                    DocumentsContract.Document.COLUMN_SIZE);

            while (cursor.moveToNext()) {
                String documentId = cursor.getString(idColumn);
                String name = cursor.getString(nameColumn);
                String mimeType = cursor.getString(mimeColumn);
                String displayPath = relativePath.isEmpty()
                        ? name
                        : relativePath + "/" + name;

                if (documentId == null || name == null) {
                    addError(errors, displayPath, "Хранилище вернуло неполные данные.");
                    continue;
                }

                if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mimeType)) {
                    collectDocuments(
                            treeUri,
                            documentId,
                            displayPath,
                            files,
                            errors,
                            visitedDirectories);
                    continue;
                }

                if (sizeColumn < 0 || cursor.isNull(sizeColumn)) {
                    addError(errors, displayPath, "Размер файла неизвестен; файл пропущен.");
                    continue;
                }

                long size = cursor.getLong(sizeColumn);
                if (size < 0) {
                    addError(errors, displayPath, "Размер файла некорректен; файл пропущен.");
                    continue;
                }

                Uri documentUri =
                        DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId);
                JSONObject file = new JSONObject();
                try {
                    file.put("uri", documentUri.toString());
                    file.put("display_path", displayPath);
                    file.put("name", name);
                    file.put("size", size);
                    files.put(file);
                } catch (JSONException error) {
                    addError(errors, displayPath, error.getMessage());
                }
            }
        } catch (RuntimeException error) {
            addError(errors, relativePath, error.getMessage());
        }
    }

    public String hashDocument(String documentUriString) {
        Uri documentUri = Uri.parse(documentUriString);
        try (InputStream input = getContentResolver().openInputStream(documentUri)) {
            if (input == null) {
                throw new IllegalStateException("Хранилище не открыло файл.");
            }
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            byte[] buffer = new byte[64 * 1024];
            int bytesRead;
            while ((bytesRead = input.read(buffer)) != -1) {
                digest.update(buffer, 0, bytesRead);
            }

            StringBuilder result = new StringBuilder(digest.getDigestLength() * 2);
            for (byte value : digest.digest()) {
                result.append(String.format(Locale.ROOT, "%02x", value & 0xff));
            }
            return result.toString();
        } catch (Exception error) {
            return "!error:" + error.getMessage();
        }
    }

    public String deleteDocument(String documentUriString) {
        try {
            boolean deleted = DocumentsContract.deleteDocument(
                    getContentResolver(), Uri.parse(documentUriString));
            return deleted ? "" : "Провайдер файлов отказал в удалении документа.";
        } catch (Exception error) {
            return "Не удалось удалить документ: " + error.getMessage();
        }
    }

    private String queryDisplayName(Uri treeUri) {
        Uri documentUri = DocumentsContract.buildDocumentUriUsingTree(
                treeUri, DocumentsContract.getTreeDocumentId(treeUri));
        String[] columns = {DocumentsContract.Document.COLUMN_DISPLAY_NAME};
        try (Cursor cursor = getContentResolver().query(documentUri, columns, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                String name = cursor.getString(0);
                if (name != null && !name.isEmpty()) {
                    return name;
                }
            }
        } catch (RuntimeException error) {
            Log.w(TAG, "Could not read the selected folder name", error);
        }
        return "Выбранная папка";
    }

    private static void addError(JSONArray errors, String path, String message) {
        if (errors.length() >= 5) {
            return;
        }
        errors.put(path + ": " + (message == null ? "неизвестная ошибка" : message));
    }

    private static String scanResult(JSONArray files, JSONArray errors, String message) {
        addError(errors, "", message);
        JSONObject result = new JSONObject();
        try {
            result.put("files", files);
            result.put("errors", errors);
            return result.toString();
        } catch (JSONException error) {
            return "{\"files\":[],\"errors\":[\"Ошибка формирования результата сканирования\"]}";
        }
    }
}
