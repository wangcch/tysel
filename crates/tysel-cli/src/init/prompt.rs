use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use dialoguer::{Confirm, Input, Select, console::Term};
use tysel_manifest::ManifestFormat;

use super::project::{validate_entry_input, validate_new_project_root};
use super::{Options, PackageJsonMode, PackageManager, Request, Template, options_from_request};

pub(super) fn configure_tty(mut request: Request) -> Result<(Options, bool)> {
    let term = Term::stdout();
    term.write_line("Create a Tysel application")?;
    term.write_line("")?;
    if request.root.is_none() {
        request.root = Some(
            if select_tty(
                &term,
                "What would you like to do?",
                &["Create a new project", "Add Tysel to the current directory"],
                0,
            )? == 0
            {
                validate_new_project_root(
                    &Input::<String>::new()
                        .with_prompt("Project directory")
                        .default("my-tysel-app".into())
                        .validate_with(|value: &String| {
                            validate_new_project_root(value)
                                .map(|_| ())
                                .map_err(|error| error.to_string())
                        })
                        .interact_text_on(&term)?,
                )?
            } else {
                PathBuf::from(".")
            },
        );
    }

    let customize = request.template.is_some()
        || request.manifest_format.is_some()
        || request.entry.is_some()
        || request.package_json.is_some()
        || request.add_scripts
        || request.package_manager.is_some()
        || request.install.is_some()
        || request.verify.is_some()
        || request.include_tests.is_some()
        || select_tty(
            &term,
            "How would you like to start?",
            &["Quick start (recommended)", "Customize"],
            0,
        )? == 1;
    let template_explicit = request.template.is_some();
    let format_explicit = request.manifest_format.is_some();
    let entry_explicit = request.entry.is_some();
    let package_explicit = request.package_json.is_some() || request.add_scripts;
    let package_manager_explicit = request.package_manager.is_some();
    let install_explicit = request.install.is_some();
    let verify_explicit = request.verify.is_some();
    let tests_explicit = request.include_tests.is_some();
    let mut options = options_from_request(request);

    if customize && !template_explicit {
        options.template = match select_tty(
            &term,
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
        options.manifest_format =
            match select_tty(&term, "Manifest format", &["TOML (recommended)", "JSON"], 0)? {
                0 => ManifestFormat::Toml,
                _ => ManifestFormat::Json,
            };
    }
    if customize && !package_explicit {
        let package_exists = options.root.join("package.json").is_file();
        if package_exists {
            match select_tty(
                &term,
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
            options.package_json = match select_tty(
                &term,
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
        let default_entry = if options.root.join("package.json").is_file() {
            "src/tysel.ts"
        } else {
            "src/index.ts"
        };
        let root = options.root.clone();
        options.entry = Some(validate_entry_input(
            &options.root,
            &Input::<String>::new()
                .with_prompt("Application entry")
                .default(default_entry.into())
                .validate_with(move |value: &String| {
                    validate_entry_input(&root, value)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                })
                .interact_text_on(&term)?,
        )?);
    }
    if customize && !tests_explicit {
        options.include_tests = confirm_on(&term, "Include tests?", true)?;
    }
    let creates_package = !options.root.join("package.json").is_file()
        && matches!(options.package_json, PackageJsonMode::Auto | PackageJsonMode::Create);
    if creates_package {
        if customize && !package_manager_explicit {
            options.package_manager = match select_tty(
                &term,
                "Package manager",
                &["npm", "pnpm", "yarn", "bun"],
                package_manager_index(options.package_manager),
            )? {
                0 => PackageManager::Npm,
                1 => PackageManager::Pnpm,
                2 => PackageManager::Yarn,
                _ => PackageManager::Bun,
            };
        }
        if !install_explicit {
            options.install = confirm_on(&term, "Install dependencies now?", false)?;
        }
        while options.install && !options.package_manager.is_available() {
            term.write_line(&format!(
                "{} was not found on PATH; choose an installed package manager.",
                options.package_manager.command()
            ))?;
            let selection = select_tty(
                &term,
                "Package manager",
                &["npm", "pnpm", "yarn", "bun", "Continue without installing"],
                package_manager_index(options.package_manager),
            )?;
            if selection == 4 {
                options.install = false;
                break;
            }
            options.package_manager = match selection {
                0 => PackageManager::Npm,
                1 => PackageManager::Pnpm,
                2 => PackageManager::Yarn,
                _ => PackageManager::Bun,
            };
        }
    }
    if !verify_explicit
        && (options.install || matches!(options.package_json, PackageJsonMode::None))
    {
        options.verify = confirm_on(&term, "Validate the generated project?", true)?;
    }
    Ok((options, true))
}

pub(super) fn confirm_tty(prompt: &str, default: bool) -> Result<bool> {
    confirm_on(&Term::stdout(), prompt, default)
}

fn select_tty(term: &Term, prompt: &str, choices: &[&str], default: usize) -> Result<usize> {
    Select::new()
        .with_prompt(prompt)
        .items(choices)
        .default(default)
        .interact_on_opt(term)?
        .ok_or_else(|| anyhow!("cancelled; no files were changed"))
}

fn confirm_on(term: &Term, prompt: &str, default: bool) -> Result<bool> {
    Confirm::new()
        .with_prompt(prompt)
        .default(default)
        .interact_on_opt(term)?
        .ok_or_else(|| anyhow!("cancelled; no files were changed"))
}

fn package_manager_index(manager: PackageManager) -> usize {
    match manager {
        PackageManager::Npm => 0,
        PackageManager::Pnpm => 1,
        PackageManager::Yarn => 2,
        PackageManager::Bun => 3,
    }
}

pub(super) fn configure<R: BufRead, W: Write>(
    mut request: Request,
    input: &mut R,
    output: &mut W,
) -> Result<(Options, bool)> {
    writeln!(output, "Create a Tysel application\n")?;
    if request.root.is_none() {
        request.root = Some(
            if prompt_select(
                input,
                output,
                "What would you like to do?",
                &["Create a new project", "Add Tysel to the current directory"],
                0,
            )? == 0
            {
                prompt_text_validated(
                    input,
                    output,
                    "Project directory",
                    "my-tysel-app",
                    validate_new_project_root,
                )?
            } else {
                PathBuf::from(".")
            },
        );
    }
    let customize = request.template.is_some()
        || request.manifest_format.is_some()
        || request.entry.is_some()
        || request.package_json.is_some()
        || request.add_scripts
        || request.package_manager.is_some()
        || request.install.is_some()
        || request.verify.is_some()
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
    let package_manager_explicit = request.package_manager.is_some();
    let install_explicit = request.install.is_some();
    let verify_explicit = request.verify.is_some();
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
        let root = options.root.clone();
        options.entry = Some(prompt_text_validated(
            input,
            output,
            "Application entry",
            default_entry,
            |value| validate_entry_input(&root, value),
        )?);
    }
    if customize && !tests_explicit {
        options.include_tests = prompt_yes_no(input, output, "Include tests?", true)?;
    }
    let creates_package = !options.root.join("package.json").is_file()
        && matches!(options.package_json, PackageJsonMode::Auto | PackageJsonMode::Create);
    if creates_package {
        if customize && !package_manager_explicit {
            options.package_manager = match prompt_select(
                input,
                output,
                "Package manager",
                &["npm", "pnpm", "yarn", "bun"],
                package_manager_index(options.package_manager),
            )? {
                0 => PackageManager::Npm,
                1 => PackageManager::Pnpm,
                2 => PackageManager::Yarn,
                _ => PackageManager::Bun,
            };
        }
        if !install_explicit {
            options.install = prompt_yes_no(input, output, "Install dependencies now?", false)?;
        }
        while options.install && !options.package_manager.is_available() {
            writeln!(
                output,
                "{} was not found on PATH; choose an installed package manager.",
                options.package_manager.command()
            )?;
            let selection = prompt_select(
                input,
                output,
                "Package manager",
                &["npm", "pnpm", "yarn", "bun", "Continue without installing"],
                package_manager_index(options.package_manager),
            )?;
            if selection == 4 {
                options.install = false;
                break;
            }
            options.package_manager = match selection {
                0 => PackageManager::Npm,
                1 => PackageManager::Pnpm,
                2 => PackageManager::Yarn,
                _ => PackageManager::Bun,
            };
        }
    }
    if !verify_explicit
        && (options.install || matches!(options.package_json, PackageJsonMode::None))
    {
        options.verify = prompt_yes_no(input, output, "Validate the generated project?", true)?;
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

pub(super) fn prompt_text_validated<R, W, T, F>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
    default: &str,
    validate: F,
) -> Result<T>
where
    R: BufRead,
    W: Write,
    F: Fn(&str) -> Result<T>,
{
    loop {
        let value = prompt_text(input, output, prompt, default)?;
        match validate(&value) {
            Ok(value) => return Ok(value),
            Err(error) => writeln!(output, "{error}")?,
        }
    }
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
