use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use tysel_manifest::ManifestFormat;

use super::{Options, PackageJsonMode, Request, Template, options_from_request};

pub(super) fn configure<R: BufRead, W: Write>(
    request: Request,
    input: &mut R,
    output: &mut W,
) -> Result<(Options, bool)> {
    writeln!(output, "Create a Tysel application\n")?;
    let customize = request.template.is_some()
        || request.manifest_format.is_some()
        || request.entry.is_some()
        || request.package_json.is_some()
        || request.add_scripts
        || request.include_tests.is_some()
        || prompt_select(
            input,
            output,
            "How would you like to start?",
            &["Quick start (recommended)", "Customize"],
            0,
        )? == 1;
    let template_explicit = request.template.is_some();
    let format_explicit = request.manifest_format.is_some();
    let entry_explicit = request.entry.is_some();
    let package_explicit = request.package_json.is_some() || request.add_scripts;
    let tests_explicit = request.include_tests.is_some();
    let mut options = options_from_request(request);
    if customize && !template_explicit {
        options.template = match prompt_select(
            input,
            output,
            "Application template",
            &["HTTP service", "Queue worker", "MCP tool", "Minimal"],
            0,
        )? {
            0 => Template::Http,
            1 => Template::Worker,
            2 => Template::Mcp,
            _ => Template::Minimal,
        };
    }
    if customize && !format_explicit {
        options.manifest_format = match prompt_select(
            input,
            output,
            "Manifest format",
            &["TOML (recommended)", "JSON"],
            0,
        )? {
            0 => ManifestFormat::Toml,
            _ => ManifestFormat::Json,
        };
    }
    if customize && !package_explicit {
        let package_exists = options.root.join("package.json").is_file();
        if package_exists {
            match prompt_select(
                input,
                output,
                "JavaScript ecosystem integration",
                &[
                    "Reuse package.json",
                    "Reuse package.json and add tysel:* scripts",
                    "Leave package.json untouched",
                ],
                0,
            )? {
                0 => options.package_json = PackageJsonMode::Reuse,
                1 => {
                    options.package_json = PackageJsonMode::Reuse;
                    options.add_scripts = true;
                }
                _ => options.package_json = PackageJsonMode::None,
            }
        } else {
            options.package_json = match prompt_select(
                input,
                output,
                "JavaScript ecosystem integration",
                &["Create package.json", "No package.json"],
                0,
            )? {
                0 => PackageJsonMode::Create,
                _ => PackageJsonMode::None,
            };
        }
    }
    if customize && !entry_explicit {
        let package_exists = options.root.join("package.json").is_file();
        let default_entry = if package_exists { "src/tysel.ts" } else { "src/index.ts" };
        options.entry =
            Some(PathBuf::from(prompt_text(input, output, "Application entry", default_entry)?));
    }
    if customize && !tests_explicit {
        options.include_tests = prompt_yes_no(input, output, "Include tests?", true)?;
    }
    Ok((options, true))
}

fn prompt_select<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
    choices: &[&str],
    default: usize,
) -> Result<usize> {
    loop {
        writeln!(output, "{prompt}")?;
        for (index, choice) in choices.iter().enumerate() {
            let marker = if index == default { "›" } else { " " };
            writeln!(output, "  {marker} {}. {choice}", index + 1)?;
        }
        write!(output, "Select [{}]: ", default + 1)?;
        output.flush()?;
        let value = read_answer(input)?;
        if value.is_empty() {
            return Ok(default);
        }
        if let Ok(index) = value.parse::<usize>()
            && (1..=choices.len()).contains(&index)
        {
            return Ok(index - 1);
        }
        writeln!(output, "Enter a number from 1 to {}.\n", choices.len())?;
    }
}

fn prompt_text<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
    default: &str,
) -> Result<String> {
    write!(output, "{prompt} [{default}]: ")?;
    output.flush()?;
    let value = read_answer(input)?;
    Ok(if value.is_empty() { default.to_owned() } else { value })
}

pub(super) fn prompt_yes_no<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
    default: bool,
) -> Result<bool> {
    loop {
        let hint = if default { "Y/n" } else { "y/N" };
        write!(output, "{prompt} [{hint}]: ")?;
        output.flush()?;
        let value = read_answer(input)?.to_ascii_lowercase();
        match value.as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(output, "Enter yes or no.")?,
        }
    }
}

fn read_answer<R: BufRead>(input: &mut R) -> Result<String> {
    let mut value = String::new();
    if input.read_line(&mut value)? == 0 {
        return Err(anyhow!("input closed; no files were changed"));
    }
    Ok(value.trim().to_owned())
}
