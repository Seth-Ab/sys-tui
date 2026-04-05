use super::*;

pub(super) fn resolve_config_path() -> Result<PathBuf> {
    if let Ok(path) = env::var("PRO_TUI_CONFIG") {
        return Ok(PathBuf::from(path));
    }

    let home = env::var("HOME").context("HOME is not set; cannot resolve config path")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("pro-tui")
        .join("config.toml"))
}

// Config loading is forgiving: invalid/missing files fall back to defaults.
pub(super) fn load_config(path: &PathBuf) -> (AppConfig, Option<String>) {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (AppConfig::default(), None),
        Err(err) => {
            return (
                AppConfig::default(),
                Some(format!("config read failed, using defaults: {err}")),
            );
        }
    };

    let parsed = match toml::from_str::<AppConfig>(&raw) {
        Ok(cfg) => cfg,
        Err(err) => {
            return (
                AppConfig::default(),
                Some(format!("config parse failed, using defaults: {err}")),
            );
        }
    };

    let (cfg, warnings) = normalize_config(parsed);
    if warnings.is_empty() {
        (cfg, None)
    } else {
        (
            cfg,
            Some(format!("config normalized: {}", warnings.join("; "))),
        )
    }
}

// Save uses a temp file then rename to avoid partially written config files.
pub(super) fn save_config(path: &PathBuf, cfg: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating config dir {}", parent.display()))?;
    }

    let body = toml::to_string_pretty(cfg)?;
    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, body)
        .with_context(|| format!("writing temp config {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path)
        .with_context(|| format!("renaming temp config to {}", path.display()))?;
    Ok(())
}

pub(super) fn normalize_config(mut cfg: AppConfig) -> (AppConfig, Vec<String>) {
    let mut warnings = Vec::new();

    let known = {
        let mut names = builtin_theme_names();
        for k in cfg.themes.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        names
    };
    if !known.iter().any(|n| n == &cfg.global.theme) {
        warnings.push(format!("theme '{}' invalid -> default", cfg.global.theme));
        cfg.global.theme = GlobalConfig::default().theme;
    }

    if color_from_name(&cfg.dashboard.assistant_color).is_none() {
        warnings.push(format!(
            "assistant_color '{}' invalid -> default",
            cfg.dashboard.assistant_color
        ));
        cfg.dashboard.assistant_color = DashboardConfig::default().assistant_color;
    }

    if color_from_name(&cfg.dashboard.user_color).is_none() {
        warnings.push(format!(
            "user_color '{}' invalid -> default",
            cfg.dashboard.user_color
        ));
        cfg.dashboard.user_color = DashboardConfig::default().user_color;
    }

    if cfg.dashboard.assistant_name.trim().is_empty() {
        warnings.push("assistant_name empty -> default".to_string());
        cfg.dashboard.assistant_name = DashboardConfig::default().assistant_name;
    }

    if cfg.dashboard.user_name.trim().is_empty() {
        warnings.push("user_name empty -> default".to_string());
        cfg.dashboard.user_name = DashboardConfig::default().user_name;
    }

    let flow_defaults = FlowMapModuleConfig::default();
    for (field, value) in [
        ("active_color", &mut cfg.modules.flow_map.active_color),
        ("run_color", &mut cfg.modules.flow_map.run_color),
        ("wait_color", &mut cfg.modules.flow_map.wait_color),
        ("ok_color", &mut cfg.modules.flow_map.ok_color),
        ("err_color", &mut cfg.modules.flow_map.err_color),
    ] {
        if color_from_name(value).is_none() {
            warnings.push(format!("modules.flow_map.{field} invalid -> default"));
            *value = match field {
                "active_color" => flow_defaults.active_color.clone(),
                "run_color" => flow_defaults.run_color.clone(),
                "wait_color" => flow_defaults.wait_color.clone(),
                "ok_color" => flow_defaults.ok_color.clone(),
                "err_color" => flow_defaults.err_color.clone(),
                _ => value.clone(),
            };
        }
    }

    let sys_defaults = SystemModuleConfig::default();
    if color_from_name(&cfg.modules.system.warn_color).is_none() {
        warnings.push("modules.system.warn_color invalid -> default".to_string());
        cfg.modules.system.warn_color = sys_defaults.warn_color;
    }
    if color_from_name(&cfg.modules.system.crit_color).is_none() {
        warnings.push("modules.system.crit_color invalid -> default".to_string());
        cfg.modules.system.crit_color = sys_defaults.crit_color;
    }

    if cfg.modules.system.memory_warn_percent >= cfg.modules.system.memory_crit_percent
        || cfg.modules.system.memory_crit_percent > 100
    {
        warnings.push("modules.system memory thresholds invalid -> defaults".to_string());
        cfg.modules.system.memory_warn_percent = sys_defaults.memory_warn_percent;
        cfg.modules.system.memory_crit_percent = sys_defaults.memory_crit_percent;
    }

    (cfg, warnings)
}
