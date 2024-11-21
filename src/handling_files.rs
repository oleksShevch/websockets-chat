use std::convert::Infallible;
use tokio::fs;
use std::path::PathBuf;
use warp::http::StatusCode;
use mime_guess;

pub async fn handle_file_download(
    file_id: String,
) -> Result<Box<dyn warp::Reply + Send>, Infallible> {
    let uploads_dir = PathBuf::from("./uploads");
    let entries = match fs::read_dir(&uploads_dir).await {
        Ok(entries) => entries,
        Err(_) => {
            return Ok(Box::new(warp::reply::with_status(
                "Uploads directory not found",
                StatusCode::NOT_FOUND,
            )))
        }
    };

    let mut entries = entries;
    while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
        let path = entry.path();
        if path.is_file() {
            if let Some(filename) = path.file_name() {
                let filename_str = filename.to_string_lossy();
                if filename_str.starts_with(&file_id) {
                    let content = match tokio::fs::read(&path).await {
                        Ok(c) => c,
                        Err(_) => {
                            return Ok(Box::new(warp::reply::with_status(
                                "Error reading file",
                                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                            )))
                        }
                    };
                    let mime_type = mime_guess::from_path(&path).first_or_octet_stream();

                    let disposition = format!(
                        "attachment; filename=\"{}\"",
                        &filename_str[file_id.len() + 1..]
                    );

                    let response = warp::reply::with_header(
                        warp::reply::with_header(
                            content,
                            "Content-Type",
                            mime_type.to_string(),
                        ),
                        "Content-Disposition",
                        disposition,
                    );
                    return Ok(Box::new(response));
                }
            }
        }
    }

    Ok(Box::new(warp::reply::with_status(
        "File not found",
        StatusCode::NOT_FOUND,
    )))
}
