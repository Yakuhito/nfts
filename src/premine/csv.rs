use std::path::{Path, PathBuf};

use crate::error::CliError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PremineRow {
    pub handle: String,
    pub recipient: String,
    pub expiration: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningRow {
    pub reason: String,
    pub source: String,
    pub nft_id: String,
    pub original_name: String,
    pub metadata_urls: String,
}

/// Write both CSVs via temp files then rename, so a failure never replaces good outputs.
pub fn write_premine_csvs_atomic(
    output: &Path,
    warnings: &Path,
    rows: &[PremineRow],
    warning_rows: &[WarningRow],
) -> Result<(), CliError> {
    let output_tmp = temp_path(output)?;
    let warnings_tmp = temp_path(warnings)?;

    write_premine_file(&output_tmp, rows)?;
    write_warnings_file(&warnings_tmp, warning_rows)?;

    std::fs::rename(&output_tmp, output).map_err(|err| {
        let _ = std::fs::remove_file(&output_tmp);
        let _ = std::fs::remove_file(&warnings_tmp);
        CliError::Io(err)
    })?;
    std::fs::rename(&warnings_tmp, warnings).map_err(|err| {
        let _ = std::fs::remove_file(&warnings_tmp);
        CliError::Io(err)
    })?;

    Ok(())
}

fn temp_path(path: &Path) -> Result<PathBuf, CliError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| CliError::Message(format!("invalid output path: {}", path.display())))?;
    Ok(parent.join(format!(".{file_name}.{}.tmp", std::process::id())))
}

fn write_premine_file(path: &Path, rows: &[PremineRow]) -> Result<(), CliError> {
    let mut out = String::from("handle,recipient,expiration\n");
    for row in rows {
        out.push_str(&escape_csv(&row.handle));
        out.push(',');
        out.push_str(&escape_csv(&row.recipient));
        out.push(',');
        out.push_str(&row.expiration.to_string());
        out.push('\n');
    }
    std::fs::write(path, out)?;
    Ok(())
}

fn write_warnings_file(path: &Path, rows: &[WarningRow]) -> Result<(), CliError> {
    let mut out = String::from("reason,source,nft_id,original_name,metadata_urls\n");
    for row in rows {
        out.push_str(&escape_csv(&row.reason));
        out.push(',');
        out.push_str(&escape_csv(&row.source));
        out.push(',');
        out.push_str(&escape_csv(&row.nft_id));
        out.push(',');
        out.push_str(&escape_csv(&row.original_name));
        out.push(',');
        out.push_str(&escape_csv(&row.metadata_urls));
        out.push('\n');
    }
    std::fs::write(path, out)?;
    Ok(())
}

fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn parse_premine_csv(contents: &str) -> Result<Vec<PremineRow>, CliError> {
    let mut lines = contents.lines();
    let header = lines
        .next()
        .ok_or_else(|| CliError::Message("premine CSV is empty".to_string()))?
        .trim();
    if header != "handle,recipient,expiration" {
        return Err(CliError::Message(format!(
            "unexpected premine CSV header: {header:?}; expected handle,recipient,expiration"
        )));
    }

    let mut rows = Vec::new();
    for (idx, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts = split_csv_line(line)?;
        if parts.len() != 3 {
            return Err(CliError::Message(format!(
                "premine CSV line {}: expected 3 columns, got {}",
                idx + 2,
                parts.len()
            )));
        }
        let expiration = parts[2].parse::<u64>().map_err(|_| {
            CliError::Message(format!(
                "premine CSV line {}: expiration must be an integer UNIX timestamp, got {:?}",
                idx + 2,
                parts[2]
            ))
        })?;
        rows.push(PremineRow {
            handle: parts[0].clone(),
            recipient: parts[1].clone(),
            expiration,
        });
    }
    Ok(rows)
}

fn split_csv_line(line: &str) -> Result<Vec<String>, CliError> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                fields.push(current);
                current = String::new();
            }
            _ => current.push(ch),
        }
    }
    if in_quotes {
        return Err(CliError::Message(
            "unterminated quote in premine CSV".to_string(),
        ));
    }
    fields.push(current);
    Ok(fields)
}
