use failure::bail;
use serde_json::{map, Value};
use std::{
    env,
    io::{self, Write},
    process,
};

type Map = map::Map<String, Value>;

type Result<T> = std::result::Result<T, failure::Error>;

fn download<F>(uri: &str, mut action: F, debug: bool) -> Result<()>
where
    F: FnMut(Map) -> Result<()>,
{
    let json: Value = reqwest::blocking::get(uri)?.json()?;
    let json = if let Value::Object(m) = json {
        m
    } else {
        bail!("Malformed JSON: {:?}", json)
    };

    if debug {
        writeln!(io::stderr(), "#json == {}", json.len())?;
        writeln!(
            io::stderr(),
            "License list version {}",
            get(&json, "licenseListVersion")?
        )?;
    }

    action(json)
}

fn get<'a>(m: &'a Map, k: &str) -> Result<&'a Value> {
    m.get(k)
        .ok_or_else(|| failure::format_err!("Malformed JSON: {:?} lacks {}", m, k))
}

const IMPRECISE: &str = include_str!("imprecise.rs");

fn is_copyleft(license: &str) -> bool {
    // Copyleft licenses are determined from
    // https://www.gnu.org/licenses/license-list.en.html
    // and no distinction is made between "weak" and "strong"
    // copyleft, for simplicity
    license.starts_with("AGPL-")
        || license.starts_with("CC-BY-NC-SA-")
        || license.starts_with("CC-BY-SA-")
        || license.starts_with("CECILL-")
        || license.starts_with("CPL-")
        || license.starts_with("CDDL-")
        || license.starts_with("EUPL")
        || license.starts_with("GFDL-")
        || license.starts_with("GPL-")
        || license.starts_with("LGPL-")
        || license.starts_with("MPL-")
        || license.starts_with("NPL-")
        || license.starts_with("OSL-")
        || license == "BSD-Protection"
        || license == "MS-PL"
        || license == "MS-RL"
        //|| license == "OpenSSL" <- this one seems to be debated, but not really copyleft
        || license == "Parity-6.0.0"
        || license == "SISSL"
        || license == "xinetd"
        || license == "YPL-1.1"
}

fn is_gnu(license: &str) -> bool {
    license.starts_with("AGPL-")
        || license.starts_with("GFDL-")
        || license.starts_with("GPL-")
        || license.starts_with("LGPL-")
}

fn real_main() -> Result<()> {
    let mut upstream_tag = None;
    let mut debug = false;
    for e in env::args().skip(1) {
        match e.as_str() {
            "-d" => {
                debug = true;
            }
            s if s.starts_with('v') => upstream_tag = Some(s.to_owned()),
            _ => bail!("Unknown option {:?}", e),
        }
    }

    let upstream_tag = match upstream_tag {
        None => {
            eprintln!(
                "WARN: fetching data from the master branch of spdx/license-list-data; \
                 consider specifying a tag (e.g. v3.0)"
            );

            "master".to_owned()
        }
        Some(ut) => {
            if debug {
                eprintln!("Using tag {:?}", ut);
            }
            ut
        }
    };

    let mut identifiers = std::fs::File::create("src/identifiers.rs")?;

    writeln!(
        identifiers,
        "\
/*
 * list fetched from https://github.com/spdx/license-list-data @ {}
 *
 * AUTO-GENERATED BY ./update
 * DO NOT MODIFY
 *
 * cargo run --manifest-path update/Cargo.toml -- v<version> > src/identifiers.rs
 */

pub const IS_FSF_LIBRE: u8 = 0x1;
pub const IS_OSI_APPROVED: u8 = 0x2;
pub const IS_DEPRECATED: u8 = 0x4;
pub const IS_COPYLEFT: u8 = 0x8;
pub const IS_GNU: u8 = 0x10;
",
        upstream_tag
    )?;

    let licenses_json_uri = format!(
        "https://raw.githubusercontent.com/spdx/license-list-data/{}/json/licenses.json",
        upstream_tag
    );

    download(
        &licenses_json_uri,
        |json| {
            let licenses = get(&json, "licenses")?;
            let licenses = if let Value::Array(ref v) = licenses {
                v
            } else {
                bail!("Malformed JSON: {:?}", licenses)
            };
            eprintln!("#licenses == {}", licenses.len());

            let mut v = vec![];
            for lic in licenses.iter() {
                let lic = if let Value::Object(ref m) = *lic {
                    m
                } else {
                    bail!("Malformed JSON: {:?}", lic)
                };
                if debug {
                    eprintln!("{:?},{:?}", get(lic, "licenseId"), get(lic, "name"));
                }

                let lic_id = get(lic, "licenseId")?;
                if let Value::String(id) = lic_id {
                    let mut flags = String::with_capacity(100);

                    if let Ok(Value::Bool(val)) = get(lic, "isDeprecatedLicenseId") {
                        if *val {
                            flags.push_str("IS_DEPRECATED | ");
                        }
                    }

                    if let Ok(Value::Bool(val)) = get(lic, "isOsiApproved") {
                        if *val {
                            flags.push_str("IS_OSI_APPROVED | ");
                        }
                    }

                    if let Ok(Value::Bool(val)) = get(lic, "isFsfLibre") {
                        if *val {
                            flags.push_str("IS_FSF_LIBRE | ");
                        }
                    }

                    if is_copyleft(id) {
                        flags.push_str("IS_COPYLEFT | ");
                    }

                    if is_gnu(id) {
                        flags.push_str("IS_GNU | ");
                    }

                    if flags.is_empty() {
                        flags.push_str("0x0");
                    } else {
                        // Strip the trailing ` | `
                        flags.truncate(flags.len() - 3);
                    }

                    let full_name = if let Value::String(name) = get(lic, "name")? {
                        name
                    } else {
                        id
                    };

                    // Add `-invariants` versions of the root GFDL-<version>
                    // licenses so that they work slightly nicer
                    if id.starts_with("GFDL-") && id.len() < 9 {
                        v.push((format!("{}-invariants", id), full_name, flags.clone()));
                    }

                    v.push((id.to_owned(), full_name, flags));
                } else {
                    bail!("Malformed JSON: {:?}", lic_id);
                }
            }

            let name = "NOASSERTION".to_owned();
            // Add NOASSERTION, which is not yet? part of the SPDX spec
            // https://github.com/spdx/spdx-spec/issues/50
            v.push(("NOASSERTION".to_owned(), &name, "0x0".to_owned()));

            v.sort_by(|a, b| a.0.cmp(&b.0));

            let lic_list_ver = get(&json, "licenseListVersion")?;
            if let Value::String(ref s) = lic_list_ver {
                writeln!(identifiers, "pub const VERSION: &str = {:?};", s)?;
            } else {
                bail!("Malformed JSON: {:?}", lic_list_ver)
            }
            writeln!(identifiers)?;
            writeln!(identifiers, "pub const LICENSES: &[(&str, &str, u8)] = &[")?;
            for (id, name, flags) in v.iter() {
                writeln!(identifiers, "    (\"{}\", r#\"{}\"#, {}),", id, name, flags)?;
            }
            writeln!(identifiers, "];")?;

            Ok(())
        },
        debug,
    )?;

    writeln!(identifiers)?;

    // Add the contents or imprecise.rs, which maps invalid identifiers to
    // valid ones
    writeln!(identifiers, "{}", IMPRECISE)?;

    let exceptions_json_uri = format!(
        "https://raw.githubusercontent.com/spdx/license-list-data/{}/json/exceptions.json",
        upstream_tag
    );

    download(
        &exceptions_json_uri,
        |json| {
            let exceptions = get(&json, "exceptions")?;
            let exceptions = if let Value::Array(ref v) = exceptions {
                v
            } else {
                bail!("Malformed JSON: {:?}", exceptions)
            };
            eprintln!("#exceptions == {}", exceptions.len());

            let mut v = vec![];
            for exc in exceptions.iter() {
                let exc = if let Value::Object(m) = exc {
                    m
                } else {
                    bail!("Malformed JSON: {:?}", exc)
                };
                if debug {
                    eprintln!(
                        "{:?},{:?}",
                        get(exc, "licenseExceptionId"),
                        get(exc, "name")
                    );
                }

                let lic_exc_id = get(exc, "licenseExceptionId")?;
                if let Value::String(s) = lic_exc_id {
                    let flags = match get(exc, "isDeprecatedLicenseId") {
                        Ok(Value::Bool(val)) => {
                            if *val {
                                "IS_DEPRECATED"
                            } else {
                                "0"
                            }
                        }
                        _ => "0",
                    };

                    v.push((s, flags));
                } else {
                    bail!("Malformed JSON: {:?}", lic_exc_id)
                };
            }

            writeln!(identifiers, "pub const EXCEPTIONS: &[(&str, u8)] = &[")?;
            v.sort_by_key(|v| v.0);
            for (exc, flags) in v.iter() {
                writeln!(identifiers, "    (\"{}\", {}),", exc, flags)?;
            }
            writeln!(identifiers, "];")?;

            Ok(())
        },
        debug,
    )?;

    drop(identifiers);

    // Run rustfmt on the final file
    std::process::Command::new("rustfmt")
        .args(&["--edition", "2018", "src/identifiers.rs"])
        .status()
        .map_err(|e| failure::format_err!("failed to run rustfmt: {}", e))?;

    Ok(())
}

fn main() {
    if let Err(ref e) = real_main() {
        eprintln!("error: {}", e);
        process::exit(1);
    }
}
