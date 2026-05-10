use super::*;
use std::fs;
use std::fs::File;
use std::io::{BufRead, Cursor, Seek, SeekFrom, Write};

pub fn extract(
    format: &Format,
    archive: &mut zip::ZipArchive<Cursor<&Vec<u8>>>,
) -> Result<HashMap<String, Vec<u8>>> {
    let fp_folder_str = format!("{}.pretty", format.name);
    let shapes_folder_str = format!("{}.3dshapes", format.name);

    //ensure we have the footprint library folder
    let footprint_folder = PathBuf::from(&format.output_path).join(fp_folder_str.clone());
    if !footprint_folder.exists() {
        fs::create_dir_all(footprint_folder.clone())?;
    }

    //ensure we have the 3D shapes folder
    let shapes_folder = PathBuf::from(&format.output_path).join(shapes_folder_str.clone());
    if !shapes_folder.exists() {
        fs::create_dir_all(shapes_folder.clone())?;
    }

    //ensure the symbol library exists
    let fn_lib = PathBuf::from(&format.output_path).join(format!("{}.kicad_sym", format.name));

    if !fn_lib.exists() {
        fs::write(
            &fn_lib,
            "(kicad_symbol_lib (version 20211014) (generator library-loader)\n)\n",
        )
        .expect("Unable to create symbol library file");
    }

    let mut symbols: Vec<String> = Vec::new();

    for i in 0..archive.len() {
        let mut item = archive.by_index(i)?;
        let name = item.name();
        let path = PathBuf::from(name);
        let base_name = path.file_name().unwrap().to_string_lossy().to_string();
        if let Some(ext) = &path.extension() {
            match ext.to_str() {
                // Footprint → .pretty/
                Some("kicad_mod") => {
                    let mut f_data = Vec::<u8>::new();
                    item.read_to_end(&mut f_data)?;
                    // Rewrite bare 3D model paths so KiCad can resolve them.
                    let raw = String::from_utf8_lossy(&f_data);
                    let fixed = rewrite_model_paths(&raw, &format.name);
                    let mut f = File::create(footprint_folder.join(base_name))?;
                    f.write_all(fixed.as_bytes())?;
                }
                // 3D model → .3dshapes/ (flat, no subfolder)
                Some("stl") | Some("stp") | Some("wrl") | Some("step") => {
                    let mut f_data = Vec::<u8>::new();
                    item.read_to_end(&mut f_data)?;
                    let mut f = File::create(shapes_folder.join(base_name))?;
                    f.write_all(&f_data)?;
                }
                Some("kicad_sym") => {
                    //save these to add later, so KiCad will be able to load the footprints right away
                    symbols.push(name.to_owned());
                }
                _ => {
                    // ignore all other files
                }
            }
        }
    }

    // Read existing content once so we can skip symbols already present.
    let existing_content = fs::read_to_string(&fn_lib).unwrap_or_default();

    let mut f = File::options().read(true).write(true).open(&fn_lib)?;
    f.seek(SeekFrom::End(-2))?;

    for symbol_file in symbols {
        let mut f_data = Vec::<u8>::new();
        let mut item = archive.by_name(&symbol_file)?;
        item.read_to_end(&mut f_data)?;
        let mut lines: Vec<String> = (&f_data[..])
            .lines()
            .map(|l| l.expect("Could not parse line"))
            .collect();
        if lines.len() < 2 {
            continue;
        }
        let end = lines.len() - 1;

        // Extract symbol name from the first content line: (symbol "NAME" ...)
        // Skip this symbol if it is already in the library.
        let sym_name: Option<&str> = lines[1]
            .trim()
            .strip_prefix("(symbol \"")
            .and_then(|s| s.find('"').map(|i| &s[..i]));
        if let Some(name) = sym_name {
            if existing_content.contains(&format!("(symbol \"{}\"", name)) {
                continue;
            }
        }

        for i in 0..end {
            //this is necessary to point symbols to correct footprint library
            let parts = lines[i].split_whitespace().collect::<Vec<_>>();
            if parts.len() >= 2 && parts[0] == "(property" && parts[1] == "\"Footprint\"" {
                let footprint_name = &parts[2][1..(parts[2].len() - 1)];
                lines[i] = lines[i].replace(
                    footprint_name,
                    &format!("{}:{}", format.name, &footprint_name),
                );
            }
        }
        for line in &lines[1..end] {
            f.write_all(line.as_bytes())?;
            f.write_all("\n".as_bytes())?;
        }
    }
    f.write_all(")\n".as_bytes())?;

    Ok(Files::new())
}

/// Rewrite bare 3D model paths in a `.kicad_mod` file so that KiCad can
/// resolve them using the `${KICAD_LOCAL_LIB_DIR}` path variable.
///
/// Handles both quoted and unquoted CSE formats:
///   `(model "foo.stp"`   →  `(model "${KICAD_LOCAL_LIB_DIR}/Lib/Lib.3dshapes/foo.stp"`
///   `(model foo.stp`     →  `(model "${KICAD_LOCAL_LIB_DIR}/Lib/Lib.3dshapes/foo.stp"`
///
/// Paths that already contain a `/`, `\`, or `$` variable are left unchanged.
fn rewrite_model_paths(content: &str, lib_name: &str) -> String {
    let mut out = String::with_capacity(content.len() + 64);
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(after_model) = trimmed.strip_prefix("(model ") {
            let indent = &line[..line.len() - trimmed.len()];

            // Quoted path: (model "foo.stp"  or  (model "foo.stp"   <rest>
            if let Some(inner) = after_model.strip_prefix('"') {
                if let Some(quote_end) = inner.find('"') {
                    let path = &inner[..quote_end];
                    let rest = &inner[quote_end..]; // starts with closing "
                    if is_bare_path(path) {
                        out.push_str(&format!(
                            "{}(model \"${{KICAD_LOCAL_LIB_DIR}}/{}/{}.3dshapes/{}",
                            indent, lib_name, lib_name, path
                        ));
                        out.push_str(rest);
                        out.push('\n');
                        continue;
                    }
                }
            } else {
                // Unquoted path: (model foo.stp   — token ends at whitespace or end-of-line
                let path_end = after_model
                    .find(|c: char| c.is_whitespace())
                    .unwrap_or(after_model.len());
                let path = &after_model[..path_end];
                let rest = &after_model[path_end..]; // trailing whitespace / empty
                if is_bare_path(path) {
                    out.push_str(&format!(
                        "{}(model \"${{KICAD_LOCAL_LIB_DIR}}/{}/{}.3dshapes/{}\"",
                        indent, lib_name, lib_name, path
                    ));
                    out.push_str(rest);
                    out.push('\n');
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Returns true for a bare filename — no directory separator and no `$` variable.
#[inline]
fn is_bare_path(path: &str) -> bool {
    !path.is_empty() && !path.contains('/') && !path.contains('\\') && !path.starts_with('$')
}
