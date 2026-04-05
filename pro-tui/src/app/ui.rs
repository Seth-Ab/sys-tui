use super::*;

pub(super) fn ui(frame: &mut ratatui::Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let palette = active_theme_palette(app);
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " pro-tui ",
            Style::default()
                .fg(palette.title_text_color)
                .bg(palette.title_color),
        ),
        Span::styled(
            format!(
                " Mac Pro dashboard for Mac mini agent  |  Screen: {}",
                App::screen_name(app.active_screen)
            ),
            Style::default().fg(palette.secondary_text_color),
        ),
    ]));
    frame.render_widget(title, outer[0]);

    match app.active_screen {
        Screen::Dashboard => render_dashboard_screen(frame, app, outer[1]),
        Screen::Customize => render_customize_screen(frame, app, outer[1]),
    }

    let mut footer_spans = vec![
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit  "),
        Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" screens  "),
        Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" close/modal back  "),
        Span::styled("h", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" help"),
    ];

    if app.active_screen == Screen::Dashboard {
        footer_spans.extend([
            Span::raw("  "),
            Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" refresh  "),
            Span::styled("e", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" expand events  "),
            Span::styled("i", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" chat input  "),
            Span::styled("m", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" models  "),
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" chat scroll"),
        ]);
    } else {
        footer_spans.extend([
            Span::raw("  "),
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" option  "),
            Span::styled("←/→", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" change value  "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" text edit/apply  "),
            Span::styled("s", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" save all  "),
            Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" reset selected  "),
            Span::styled("t", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" toggle live preview  "),
            Span::styled("[ / ]", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" section"),
        ]);
    }

    let footer = Paragraph::new(Line::from(footer_spans))
        .style(Style::default().fg(palette.secondary_text_color));
    frame.render_widget(footer, outer[2]);

    if app.show_help {
        render_help_popup(frame, app);
    }

    if app.show_model_selector {
        render_model_selector_popup(frame, app);
    }

    if app.show_navigator {
        render_navigator_popup(frame, app);
    }

    if app.show_color_picker {
        render_color_picker_popup(frame, app);
    }
}

fn render_dashboard_screen(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    match app.config.dashboard.layout_preset {
        DashboardLayoutPreset::ThreeTopTwoBottom => {
            render_dashboard_three_top_two_bottom(frame, app, area)
        }
        DashboardLayoutPreset::LlmColumn => render_dashboard_llm_column(frame, app, area),
    }
}

fn render_dashboard_three_top_two_bottom(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let palette = active_theme_palette(app);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    let top_constraints = if app.config.dashboard.show_processes {
        vec![Constraint::Percentage(75), Constraint::Percentage(25)]
    } else {
        vec![Constraint::Percentage(100)]
    };
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(top_constraints)
        .split(body[0]);

    let top_left = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top[0]);

    if app.config.dashboard.show_status {
        frame.render_widget(
            Paragraph::new(status_lines(app))
                .block(block("Status", app))
                .style(Style::default().fg(palette.main_text_color))
                .wrap(Wrap { trim: true }),
            top_left[0],
        );
    } else {
        frame.render_widget(
            Paragraph::new("hidden (toggle in Customize)")
                .style(Style::default().fg(palette.main_text_color))
                .block(block("Status", app)),
            top_left[0],
        );
    }

    if app.config.dashboard.show_system {
        frame.render_widget(
            Paragraph::new(system_lines(app))
                .block(block("System", app))
                .style(Style::default().fg(palette.main_text_color))
                .wrap(Wrap { trim: true }),
            top_left[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new("hidden (toggle in Customize)")
                .style(Style::default().fg(palette.main_text_color))
                .block(block("System", app)),
            top_left[1],
        );
    }

    if app.config.dashboard.show_processes && top.len() > 1 {
        frame.render_widget(
            Paragraph::new(process_lines(app))
                .block(block("Top Processes", app))
                .style(Style::default().fg(palette.main_text_color))
                .wrap(Wrap { trim: true }),
            top[1],
        );
    }

    let bottom = match (
        app.config.dashboard.show_events,
        app.config.dashboard.show_flow,
    ) {
        (true, true) => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(52),
                Constraint::Percentage(24),
                Constraint::Percentage(24),
            ])
            .split(body[1]),
        (true, false) | (false, true) => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(body[1]),
        (false, false) => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(body[1]),
    };

    render_llm_panel(frame, app, bottom[0]);

    match (
        app.config.dashboard.show_events,
        app.config.dashboard.show_flow,
    ) {
        (true, true) => {
            frame.render_widget(
                Paragraph::new(event_lines(app))
                    .block(block("File Events", app))
                    .style(Style::default().fg(palette.main_text_color))
                    .wrap(Wrap { trim: true }),
                bottom[1],
            );
            frame.render_widget(
                Paragraph::new(flow_lines(app))
                    .block(block("Flow Map", app))
                    .style(Style::default().fg(palette.main_text_color))
                    .wrap(Wrap { trim: true }),
                bottom[2],
            );
        }
        (true, false) => {
            frame.render_widget(
                Paragraph::new(event_lines(app))
                    .block(block("File Events", app))
                    .style(Style::default().fg(palette.main_text_color))
                    .wrap(Wrap { trim: true }),
                bottom[1],
            );
        }
        (false, true) => {
            frame.render_widget(
                Paragraph::new(flow_lines(app))
                    .block(block("Flow Map", app))
                    .style(Style::default().fg(palette.main_text_color))
                    .wrap(Wrap { trim: true }),
                bottom[1],
            );
        }
        (false, false) => {}
    }
}

fn render_dashboard_llm_column(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let palette = active_theme_palette(app);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(67)])
        .split(area);

    render_llm_panel(frame, app, columns[0]);

    let show_bottom = app.config.dashboard.show_events || app.config.dashboard.show_flow;
    let right = if show_bottom {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(columns[1])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(100)])
            .split(columns[1])
    };

    let top_constraints = if app.config.dashboard.show_processes {
        vec![Constraint::Percentage(70), Constraint::Percentage(30)]
    } else {
        vec![Constraint::Percentage(100)]
    };
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(top_constraints)
        .split(right[0]);

    let top_left = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top[0]);

    if app.config.dashboard.show_status {
        frame.render_widget(
            Paragraph::new(status_lines(app))
                .block(block("Status", app))
                .style(Style::default().fg(palette.main_text_color))
                .wrap(Wrap { trim: true }),
            top_left[0],
        );
    } else {
        frame.render_widget(
            Paragraph::new("hidden (toggle in Customize)")
                .style(Style::default().fg(palette.main_text_color))
                .block(block("Status", app)),
            top_left[0],
        );
    }

    if app.config.dashboard.show_system {
        frame.render_widget(
            Paragraph::new(system_lines(app))
                .block(block("System", app))
                .style(Style::default().fg(palette.main_text_color))
                .wrap(Wrap { trim: true }),
            top_left[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new("hidden (toggle in Customize)")
                .style(Style::default().fg(palette.main_text_color))
                .block(block("System", app)),
            top_left[1],
        );
    }

    if app.config.dashboard.show_processes && top.len() > 1 {
        frame.render_widget(
            Paragraph::new(process_lines(app))
                .block(block("Top Processes", app))
                .style(Style::default().fg(palette.main_text_color))
                .wrap(Wrap { trim: true }),
            top[1],
        );
    }

    if show_bottom && right.len() > 1 {
        let bottom = match (
            app.config.dashboard.show_events,
            app.config.dashboard.show_flow,
        ) {
            (true, true) => Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(right[1]),
            _ => Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(100)])
                .split(right[1]),
        };

        match (
            app.config.dashboard.show_events,
            app.config.dashboard.show_flow,
        ) {
            (true, true) => {
                frame.render_widget(
                    Paragraph::new(event_lines(app))
                        .block(block("File Events", app))
                        .style(Style::default().fg(palette.main_text_color))
                        .wrap(Wrap { trim: true }),
                    bottom[0],
                );
                frame.render_widget(
                    Paragraph::new(flow_lines(app))
                        .block(block("Flow Map", app))
                        .style(Style::default().fg(palette.main_text_color))
                        .wrap(Wrap { trim: true }),
                    bottom[1],
                );
            }
            (true, false) => {
                frame.render_widget(
                    Paragraph::new(event_lines(app))
                        .block(block("File Events", app))
                        .style(Style::default().fg(palette.main_text_color))
                        .wrap(Wrap { trim: true }),
                    bottom[0],
                );
            }
            (false, true) => {
                frame.render_widget(
                    Paragraph::new(flow_lines(app))
                        .block(block("Flow Map", app))
                        .style(Style::default().fg(palette.main_text_color))
                        .wrap(Wrap { trim: true }),
                    bottom[0],
                );
            }
            (false, false) => {}
        }
    }
}

fn render_customize_screen(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let palette = active_theme_palette(app);
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    let mut section_lines = vec![Line::raw("Sections"), Line::raw("")];
    for (idx, section) in App::customize_sections().iter().enumerate() {
        let prefix = if idx == app.customize_section_idx {
            ">"
        } else {
            " "
        };
        let label = match section {
            CustomizeSection::Global => "Global",
            CustomizeSection::Dashboard => "Dashboard",
            CustomizeSection::Themes => "New Themes",
        };
        section_lines.push(if idx == app.customize_section_idx {
            Line::from(vec![
                Span::raw(format!("{prefix} ")),
                Span::styled(
                    label,
                    Style::default()
                        .fg(palette.focus_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        } else {
            Line::raw(format!("{prefix} {label}"))
        });
    }

    frame.render_widget(
        Paragraph::new(section_lines)
            .block(block("Customize", app))
            .style(Style::default().fg(palette.main_text_color))
            .wrap(Wrap { trim: true }),
        layout[0],
    );

    let mut option_lines: Vec<Line<'static>> = vec![
        Line::raw("Use [ and ] to switch section".to_string()),
        Line::raw("Up/Down select option, Left/Right change value".to_string()),
        Line::raw("Enter: edit/apply current option, s: save all, r: reset selected".to_string()),
        Line::raw("".to_string()),
    ];

    match app.active_customize_section() {
        CustomizeSection::Global => {
            for (idx, option) in App::global_options().iter().enumerate() {
                let selected = idx == app.global_option_idx;
                let value = match option {
                    GlobalOption::Theme => app.config.global.theme.clone(),
                };
                option_lines.push(customize_option_line(
                    selected,
                    false,
                    "theme",
                    &value,
                    palette.focus_color,
                ));
            }
        }
        CustomizeSection::Dashboard => {
            for (idx, option) in App::dashboard_options().iter().enumerate() {
                let selected = idx == app.dashboard_option_idx;
                let (name, value) = match option {
                    DashboardOption::AssistantName => (
                        "assistant_name",
                        app.config.dashboard.assistant_name.clone(),
                    ),
                    DashboardOption::AssistantColor => (
                        "assistant_color",
                        app.config.dashboard.assistant_color.clone(),
                    ),
                    DashboardOption::UserName => {
                        ("user_name", app.config.dashboard.user_name.clone())
                    }
                    DashboardOption::UserColor => {
                        ("user_color", app.config.dashboard.user_color.clone())
                    }
                    DashboardOption::LayoutPreset => (
                        "layout_preset",
                        layout_preset_label(app.config.dashboard.layout_preset).to_string(),
                    ),
                    DashboardOption::ShowStatus => (
                        "show_status",
                        bool_label(app.config.dashboard.show_status).to_string(),
                    ),
                    DashboardOption::ShowSystem => (
                        "show_system",
                        bool_label(app.config.dashboard.show_system).to_string(),
                    ),
                    DashboardOption::ShowProcesses => (
                        "show_processes",
                        bool_label(app.config.dashboard.show_processes).to_string(),
                    ),
                    DashboardOption::ShowEvents => (
                        "show_events",
                        bool_label(app.config.dashboard.show_events).to_string(),
                    ),
                    DashboardOption::ShowFlow => (
                        "show_flow_map",
                        bool_label(app.config.dashboard.show_flow).to_string(),
                    ),
                };
                let editing = app.customize_text_mode
                    && selected
                    && matches!(
                        option,
                        DashboardOption::AssistantName | DashboardOption::UserName
                    );
                let display_value = if editing {
                    format!("{}_", app.customize_text_buffer)
                } else {
                    value
                };

                option_lines.push(customize_option_line(
                    selected,
                    editing,
                    name,
                    &display_value,
                    palette.focus_color,
                ));
            }
        }
        CustomizeSection::Themes => {
            option_lines.push(Line::raw(
                "Create a new theme using ANSI-256 colors".to_string(),
            ));
            option_lines.push(Line::raw(
                "Pick colors, choose a name, then Save New Theme".to_string(),
            ));
            option_lines.push(Line::raw(format!(
                "live_preview: {} (press t to toggle)",
                bool_label(app.theme_draft_live_preview)
            )));
            option_lines.push(Line::raw("".to_string()));

            for (idx, opt) in app.themes_options().iter().enumerate() {
                let selected = idx == app.themes_option_idx;
                let prefix = if selected { "> " } else { "  " };
                let value_style = if selected {
                    Style::default()
                        .fg(palette.focus_color)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.main_text_color)
                };

                match opt {
                    ThemesOption::NewThemeName => {
                        let editing = app.customize_text_mode
                            && selected
                            && app.customize_text_target == Some(CustomizeTextTarget::ThemeNewName);
                        let value = if editing {
                            format!("{}_", app.customize_text_buffer)
                        } else {
                            app.theme_new_name.clone()
                        };
                        option_lines.push(Line::from(vec![
                            Span::raw(prefix.to_string()),
                            Span::styled("new_theme_name: ".to_string(), value_style),
                            Span::styled(value, value_style),
                        ]));
                    }
                    ThemesOption::SaveNewTheme => {
                        option_lines.push(Line::from(vec![
                            Span::raw(prefix.to_string()),
                            Span::styled("save_new_theme".to_string(), value_style),
                        ]));
                    }

                    ThemesOption::ColorPick(field) => {
                        let c = app.theme_color_for_editor(*field).unwrap_or_default();
                        let ansi_name = base_palette_name(nearest_base_palette_index(c));
                        let swatch = Span::styled(
                            "  ██  ".to_string(),
                            Style::default()
                                .bg(c.to_display_color())
                                .fg(c.to_display_color()),
                        );
                        option_lines.push(Line::from(vec![
                            Span::raw(prefix.to_string()),
                            Span::styled(
                                format!("{}: ", theme_color_field_name(*field)),
                                value_style,
                            ),
                            Span::styled(ansi_name.to_string(), value_style),
                            Span::raw(" ".to_string()),
                            swatch,
                        ]));
                    }
                }
            }
        }
    }

    option_lines.push(Line::raw("".to_string()));
    option_lines.push(Line::raw(format!(
        "Config path: {}",
        app.config_path.display()
    )));
    option_lines.push(Line::raw(format!(
        "Dirty: {}",
        bool_label(app.config_dirty)
    )));

    if let Some(msg) = app.visible_status_message() {
        option_lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(msg.to_string()),
        ]));
    }

    frame.render_widget(
        Paragraph::new(option_lines)
            .block(block("Options", app))
            .style(Style::default().fg(palette.main_text_color))
            .wrap(Wrap { trim: true }),
        layout[1],
    );
}

fn render_color_picker_popup(frame: &mut ratatui::Frame, app: &App) {
    let popup = centered_rect(78, 34, frame.area());
    frame.render_widget(Clear, popup);

    let Some(field) = app.color_picker_field else {
        return;
    };

    let base_idx = app.color_picker_idx.min(base_palette_len().saturating_sub(1));
    let primary_idx = app.color_picker_primary_idx.min(15);
    let extended_idx = app
        .color_picker_extended_idx
        .max(16)
        .min(base_palette_len().saturating_sub(1));
    let shade_idx = app.color_picker_shade_idx.min(15);
    let shades = shade_gradient_for_base(base_idx);
    let current = match app.color_picker_row {
        ColorPickerRow::BasePrimary => base_palette_color_at(primary_idx),
        ColorPickerRow::BaseExtended => base_palette_color_at(extended_idx),
        ColorPickerRow::Shade => shades[shade_idx],
    };

    let mut base_row_1: Vec<Span<'static>> = Vec::new();
    let mut base_name_row_1: Vec<Span<'static>> = Vec::new();
    for idx in 0..16 {
        let selected = app.color_picker_row == ColorPickerRow::BasePrimary && idx == primary_idx;
        let swatch_color = base_palette_color_at(idx).to_display_color();
        let style = if selected {
            Style::default()
                .bg(swatch_color)
                .fg(swatch_color)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().bg(swatch_color).fg(swatch_color)
        };
        base_row_1.push(Span::styled("██".to_string(), style));
        base_row_1.push(Span::raw(" ".to_string()));

        let name_style = if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        base_name_row_1.push(Span::styled(format!("{} ", base_palette_short(idx)), name_style));
    }

    let mut base_row_2: Vec<Span<'static>> = Vec::new();
    let mut base_name_row_2: Vec<Span<'static>> = Vec::new();
    for idx in 16..32 {
        let selected = app.color_picker_row == ColorPickerRow::BaseExtended && idx == extended_idx;
        let swatch_color = base_palette_color_at(idx).to_display_color();
        let style = if selected {
            Style::default()
                .bg(swatch_color)
                .fg(swatch_color)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().bg(swatch_color).fg(swatch_color)
        };
        base_row_2.push(Span::styled("██".to_string(), style));
        base_row_2.push(Span::raw(" ".to_string()));

        let name_style = if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        base_name_row_2.push(Span::styled(format!("{} ", base_palette_short(idx)), name_style));
    }

    let mut shade_row: Vec<Span<'static>> = Vec::new();
    let mut shade_label_row: Vec<Span<'static>> = Vec::new();
    for (idx, shade) in shades.iter().enumerate() {
        let selected = app.color_picker_row == ColorPickerRow::Shade && idx == shade_idx;
        let swatch_color = shade.to_display_color();
        let style = if selected {
            Style::default()
                .bg(swatch_color)
                .fg(swatch_color)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().bg(swatch_color).fg(swatch_color)
        };
        shade_row.push(Span::styled("██".to_string(), style));
        shade_row.push(Span::raw(" ".to_string()));

        let label_style = if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        shade_label_row.push(Span::styled(format!("{:02} ", idx + 1), label_style));
    }

    let rendered_name = match app.color_picker_row {
        ColorPickerRow::BasePrimary => base_palette_name(primary_idx),
        ColorPickerRow::BaseExtended => base_palette_name(extended_idx),
        ColorPickerRow::Shade => {
            format!("{} shade {:02}", base_palette_name(base_idx), shade_idx + 1)
        }
    };

    let lines = vec![
        Line::raw(
            "ANSI-256 Color Picker (Up/Down row, Left/Right move, Enter apply, Esc cancel)",
        ),
        Line::raw("Row 1: vivid ANSI-256 base colors"),
        Line::raw("Row 2: extra ANSI-256 base colors"),
        Line::raw("Row 3: 16 shades around selected base"),
        Line::raw(""),
        Line::from(base_row_1),
        Line::from(base_name_row_1),
        Line::raw(""),
        Line::from(base_row_2),
        Line::from(base_name_row_2),
        Line::raw(""),
        Line::from(shade_row),
        Line::from(shade_label_row),
        Line::raw(""),
        Line::raw(format!("Field: {}", theme_color_field_name(field))),
        Line::raw(format!("Rendered as: {}", rendered_name)),
        Line::raw(format!("RGB: {}, {}, {}", current.r, current.g, current.b)),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("Color Picker", app))
            .style(Style::default().fg(active_theme_palette(app).main_text_color))
            .wrap(Wrap { trim: true }),
        popup,
    );
}
fn render_navigator_popup(frame: &mut ratatui::Frame, app: &App) {
    let palette = active_theme_palette(app);
    let popup = centered_rect(45, 45, frame.area());
    frame.render_widget(Clear, popup);

    let mut lines = vec![
        Line::raw("Navigator (Enter select, Esc close)"),
        Line::raw(""),
    ];

    for (idx, screen) in App::screens().iter().enumerate() {
        let hover = if idx == app.navigator_hover_idx {
            ">"
        } else {
            " "
        };
        let active = if *screen == app.active_screen {
            "*"
        } else {
            " "
        };
        let label = App::screen_name(*screen);
        lines.push(Line::from(vec![
            Span::raw(format!("{}{} ", hover, active)),
            if idx == app.navigator_hover_idx {
                Span::styled(
                    label,
                    Style::default()
                        .fg(palette.focus_color)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw(label)
            },
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("Screens", app))
            .style(Style::default().fg(palette.main_text_color))
            .wrap(Wrap { trim: true }),
        popup,
    );
}

fn render_help_popup(frame: &mut ratatui::Frame, app: &App) {
    let popup = centered_rect(78, 78, frame.area());
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw("pro-tui help"),
            Line::raw(""),
            Line::raw("Global"),
            Line::raw("q          quit"),
            Line::raw("Tab        open screen navigator"),
            Line::raw("h          open help"),
            Line::raw("Esc        close active modal/input"),
            Line::raw(""),
            Line::raw(format!(
                "Current screen: {}",
                App::screen_name(app.active_screen)
            )),
            Line::raw(""),
            Line::raw("Dashboard"),
            Line::raw("r          refresh immediately"),
            Line::raw("e          expand/collapse file events"),
            Line::raw("i          focus chat input"),
            Line::raw("m          open model selector"),
            Line::raw("Up/Down    scroll chat"),
            Line::raw("input focus: Chat + Input use focus_color"),
            Line::raw(""),
            Line::raw("Model Selector Popup"),
            Line::raw("Up/Down    move selection"),
            Line::raw("Enter      apply model"),
            Line::raw("Esc        close selector"),
            Line::raw(""),
            Line::raw("Screen Navigator Popup"),
            Line::raw("Up/Down    move"),
            Line::raw("Enter      switch screen"),
            Line::raw("Esc        close navigator"),
            Line::raw(""),
            Line::raw("Customize"),
            Line::raw("[ / ]      switch section (Global/Dashboard/New Themes)"),
            Line::raw("Up/Down    move option"),
            Line::raw("Left/Right change value"),
            Line::raw("t          toggle live preview (New Themes section)"),
            Line::raw("Enter      edit/apply selected option"),
            Line::raw("r          reset selected option to default"),
            Line::raw("s          save all config changes"),
            Line::raw("Esc        cancel inline text edit / close color picker"),
            Line::raw(""),
            Line::raw("New Themes Color Picker (ANSI-256)"),
            Line::raw("Up/Down    move row (Base-1, Base-2, Shade)"),
            Line::raw("Left/Right move selection"),
            Line::raw("Enter      apply picked color"),
            Line::raw("Esc        cancel picker"),
            Line::raw(""),
            Line::raw("Notes"),
            Line::raw("- Dashboard assistant/user colors override theme for chat role labels."),
            Line::raw("- Flow Map and System can colorize states/thresholds via module config."),
        ])
        .block(block("Help", app))
        .style(Style::default().fg(active_theme_palette(app).main_text_color))
        .wrap(Wrap { trim: true }),
        popup,
    );
}

fn render_model_selector_popup(frame: &mut ratatui::Frame, app: &App) {
    let palette = active_theme_palette(app);
    let popup = centered_rect(65, 70, frame.area());
    frame.render_widget(Clear, popup);

    let running: HashSet<&str> = app.running_models.iter().map(String::as_str).collect();
    let effective = app.selected_model.as_deref();

    let mut lines = vec![
        Line::raw("Select model (Enter apply, Esc close)"),
        Line::raw(""),
    ];

    for (idx, model) in app.model_list.iter().enumerate() {
        let prefix = if idx == app.model_hover_idx { ">" } else { " " };
        let run_mark = if running.contains(model.as_str()) {
            "[running]"
        } else {
            "         "
        };
        let sel_mark = if effective == Some(model.as_str()) {
            "*"
        } else {
            " "
        };

        lines.push(Line::from(vec![
            Span::raw(format!("{prefix}{sel_mark} {run_mark} ")),
            if idx == app.model_hover_idx {
                Span::styled(
                    model.clone(),
                    Style::default()
                        .fg(palette.focus_color)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw(model.clone())
            },
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block("Models", app))
            .style(Style::default().fg(palette.main_text_color))
            .wrap(Wrap { trim: true }),
        popup,
    );
}

fn render_llm_panel(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let palette = active_theme_palette(app);
    let outer = block("LLM", app).inner(area);
    frame.render_widget(block("LLM", app), area);

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(outer);

    let llm_summary = Paragraph::new(llm_lines(app))
        .style(Style::default().fg(palette.main_text_color))
        .wrap(Wrap { trim: true });
    frame.render_widget(llm_summary, parts[0]);

    let chat_block = if app.input_mode {
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.focus_color))
            .title(Span::styled(
                " Chat ".to_string(),
                Style::default()
                    .fg(palette.focus_color)
                    .add_modifier(Modifier::BOLD),
            ))
    } else {
        block("Chat", app)
    };

    let chat_window = Paragraph::new(chat_lines(app))
        .block(chat_block)
        .style(Style::default().fg(palette.main_text_color))
        .scroll((app.chat_scroll.min(u16::MAX as usize) as u16, 0))
        .wrap(Wrap { trim: true });
    frame.render_widget(chat_window, parts[1]);

    let input_title = if app.input_mode {
        "Input (focused: Enter send, Esc stop)"
    } else {
        "Input (press i to focus)"
    };
    let input_style = if app.input_mode {
        Style::default().fg(palette.focus_color)
    } else {
        Style::default().fg(palette.main_text_color)
    };

    let input = Paragraph::new(format!("> {}", app.chat_input))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border_color))
                .title(Span::styled(
                    input_title.to_string(),
                    Style::default()
                        .fg(palette.section_title_color)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .style(input_style)
        .wrap(Wrap { trim: true });
    frame.render_widget(input, parts[2]);
}

fn block(title: &'static str, app: &App) -> Block<'static> {
    let palette = active_theme_palette(app);
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border_color))
        .title(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(palette.section_title_color)
                .add_modifier(Modifier::BOLD),
        ))
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

fn status_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::raw(format!("endpoint: {}", app.endpoint)),
        Line::raw(format!(
            "last fetch: {}",
            app.last_fetch
                .map(|t| format!("{} ms ago", t.elapsed().as_millis()))
                .unwrap_or_else(|| "never".to_string())
        )),
    ];

    if let Some(state) = &app.state {
        lines.push(Line::raw(format!("host: {}", state.hostname)));
        lines.push(Line::raw(format!("seq: {}", state.seq)));
        lines.push(Line::raw(format!("ts_ms: {}", state.ts_ms)));
    } else {
        lines.push(Line::raw("host: (no data)".to_string()));
    }

    match &app.last_error {
        Some(err) => lines.push(Line::raw(format!("error: {err}"))),
        None => lines.push(Line::raw("error: none".to_string())),
    }

    lines
}

fn system_lines(app: &App) -> Vec<Line<'static>> {
    let Some(state) = &app.state else {
        return vec![Line::raw("waiting for first snapshot...".to_string())];
    };

    let s = &state.system;
    let module_cfg = &app.config.modules.system;
    let mem_percent = if s.memory_total_bytes > 0 {
        (s.memory_used_bytes as f64 / s.memory_total_bytes as f64) * 100.0
    } else {
        0.0
    };

    let mem_style = if module_cfg.colorize {
        if mem_percent >= f64::from(module_cfg.memory_crit_percent) {
            Style::default().fg(
                color_from_name(&module_cfg.crit_color).unwrap_or(Color::LightRed),
            )
        } else if mem_percent >= f64::from(module_cfg.memory_warn_percent) {
            Style::default().fg(
                color_from_name(&module_cfg.warn_color).unwrap_or(Color::Yellow),
            )
        } else {
            Style::default()
        }
    } else {
        Style::default()
    };

    vec![
        Line::raw(format!("cpu: {:.1}%", s.cpu_percent)),
        Line::from(vec![Span::styled(
            format!(
                "mem: {} / {} ({:.0}%)",
                bytes_human(s.memory_used_bytes),
                bytes_human(s.memory_total_bytes),
                mem_percent
            ),
            mem_style,
        )]),
        Line::raw(format!(
            "swap: {} / {}",
            bytes_human(s.swap_used_bytes),
            bytes_human(s.swap_total_bytes)
        )),
        Line::raw(format!(
            "root: {} / {}",
            bytes_human(s.root_used_bytes),
            bytes_human(s.root_total_bytes)
        )),
    ]
}

fn llm_lines(app: &App) -> Vec<Line<'static>> {
    let Some(state) = &app.state else {
        return vec![Line::raw("waiting for first snapshot...".to_string())];
    };

    let llm = &state.llm;
    let selected = app
        .selected_model
        .clone()
        .or_else(|| state.llm.running_models.first().cloned())
        .unwrap_or_else(|| "(none)".to_string());

    let mut lines = vec![
        Line::raw(format!("ollama endpoint: {}", llm.ollama_ps_url)),
        Line::raw(format!("online: {}", llm.ollama_online)),
        Line::raw(format!("running models: {}", llm.model_count)),
        Line::raw(format!("selected chat model: {selected}")),
    ];

    if let Some(err) = &llm.error {
        lines.push(Line::raw(format!("error: {err}")));
    }

    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatLineKind {
    Plain,
    Code,
    CodeLang,
}

#[derive(Debug, Clone)]
struct ChatLine {
    text: String,
    kind: ChatLineKind,
}

fn chat_lines(app: &App) -> Vec<Line<'static>> {
    if app.chat_history.is_empty() {
        return vec![Line::raw(
            "No chat messages yet. Press i to type, m to choose model.".to_string(),
        )];
    }

    let mut out = Vec::new();
    for msg in app.chat_history.iter().rev().take(20).rev() {
        let formatted = format_markdown_for_chat(&msg.content);
        let (label, prefix_style) = role_display(app, &msg.role);

        if formatted.is_empty() {
            out.push(Line::from(vec![Span::styled(
                format!("{}:", label),
                prefix_style,
            )]));
            continue;
        }

        for (idx, line) in formatted.iter().enumerate() {
            let first_prefix = format!("{}: ", label);
            let cont_prefix = " ".repeat(label.len() + 2);
            let prefix = if idx == 0 {
                first_prefix.as_str()
            } else {
                cont_prefix.as_str()
            };

            let styled = match line.kind {
                ChatLineKind::Plain => {
                    plain_line_with_inline_code(prefix, prefix_style, &line.text)
                }
                ChatLineKind::Code => Line::from(vec![
                    Span::styled(prefix.to_string(), prefix_style),
                    Span::styled(line.text.clone(), Style::default().fg(Color::Cyan)),
                ]),
                ChatLineKind::CodeLang => Line::from(vec![
                    Span::styled(prefix.to_string(), prefix_style),
                    Span::styled(
                        line.text.clone(),
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
            };
            out.push(styled);
        }
    }

    out
}

// Tiny markdown-ish formatter for terminal chat readability.
fn format_markdown_for_chat(text: &str) -> Vec<ChatLine> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut in_code = false;

    while i < text.len() {
        let rem = &text[i..];
        if let Some(rel) = rem.find("```") {
            let pos = i + rel;
            let seg = &text[i..pos];
            append_segment(seg, in_code, &mut out);

            i = pos + 3;
            if in_code {
                in_code = false;
                out.push(ChatLine {
                    text: String::new(),
                    kind: ChatLineKind::Plain,
                });
            } else {
                let (lang, consumed) = parse_fence_lang(&text[i..]);
                i += consumed;
                in_code = true;
                out.push(ChatLine {
                    text: String::new(),
                    kind: ChatLineKind::Plain,
                });
                if let Some(lang) = lang {
                    out.push(ChatLine {
                        text: format!("[{}]", lang),
                        kind: ChatLineKind::CodeLang,
                    });
                }
            }
        } else {
            let seg = &text[i..];
            append_segment(seg, in_code, &mut out);
            break;
        }
    }

    while out
        .first()
        .is_some_and(|l| l.text.is_empty() && l.kind == ChatLineKind::Plain)
    {
        out.remove(0);
    }
    while out
        .last()
        .is_some_and(|l| l.text.is_empty() && l.kind == ChatLineKind::Plain)
    {
        out.pop();
    }

    out
}

fn parse_fence_lang(input: &str) -> (Option<String>, usize) {
    if input.is_empty() {
        return (None, 0);
    }

    let mut lang = String::new();
    let mut consumed = 0usize;

    for ch in input.chars() {
        if ch == '\n' {
            consumed += ch.len_utf8();
            break;
        }
        if ch.is_whitespace() {
            consumed += ch.len_utf8();
            break;
        }
        lang.push(ch);
        consumed += ch.len_utf8();
    }

    let lang = if lang.is_empty() { None } else { Some(lang) };
    (lang, consumed)
}

fn append_segment(seg: &str, in_code: bool, out: &mut Vec<ChatLine>) {
    if seg.is_empty() {
        return;
    }

    for line in seg.split('\n') {
        if in_code {
            out.push(ChatLine {
                text: format!("  | {}", line),
                kind: ChatLineKind::Code,
            });
        } else {
            out.push(ChatLine {
                text: line.to_string(),
                kind: ChatLineKind::Plain,
            });
        }
    }
}

fn plain_line_with_inline_code(prefix: &str, prefix_style: Style, text: &str) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(Span::styled(prefix.to_string(), prefix_style));

    let mut buf = String::new();
    let mut in_inline_code = false;

    for ch in text.chars() {
        if ch == '`' {
            if !buf.is_empty() {
                if in_inline_code {
                    spans.push(Span::styled(buf.clone(), Style::default().fg(Color::Cyan)));
                } else {
                    spans.push(Span::raw(buf.clone()));
                }
                buf.clear();
            }
            in_inline_code = !in_inline_code;
            continue;
        }
        buf.push(ch);
    }

    if !buf.is_empty() {
        if in_inline_code {
            spans.push(Span::styled(buf, Style::default().fg(Color::Cyan)));
        } else {
            spans.push(Span::raw(buf));
        }
    }

    Line::from(spans)
}

fn process_lines(app: &App) -> Vec<Line<'static>> {
    let Some(state) = &app.state else {
        return vec![Line::raw("waiting for first snapshot...".to_string())];
    };

    if state.system.top_processes.is_empty() {
        return vec![Line::raw("no process data".to_string())];
    }

    let mut lines = vec![Line::raw("ranked by avg CPU over last 5m".to_string())];
    lines.extend(state.system.top_processes.iter().map(|p| {
        Line::raw(format!(
            "pid={} avg5m={:.1}% now={:.1}% n={} mem={} {}",
            p.pid,
            p.cpu_percent,
            p.current_cpu_percent,
            p.samples_5m,
            bytes_human(p.memory_bytes),
            p.name
        ))
    }));
    lines
}

fn event_lines(app: &App) -> Vec<Line<'static>> {
    let Some(state) = &app.state else {
        return vec![Line::raw("waiting for first snapshot...".to_string())];
    };

    let mut lines = vec![];
    lines.push(Line::raw(format!(
        "watching: {}",
        if state.watched_dirs.is_empty() {
            "(none)".to_string()
        } else {
            state.watched_dirs.join(", ")
        }
    )));

    let limit = if app.expanded_events { 20 } else { 8 };
    if state.recent_file_events.is_empty() {
        lines.push(Line::raw("no recent file events".to_string()));
    } else {
        lines.extend(
            state
                .recent_file_events
                .iter()
                .take(limit)
                .map(|e| Line::raw(e.clone())),
        );
    }

    lines
}

fn flow_lines(app: &App) -> Vec<Line<'static>> {
    let flow_cfg = &app.config.modules.flow_map;
    let palette = active_theme_palette(app);
    let pending = app.chat_history.iter().any(|m| m.id.is_some());
    let chat_error = app
        .chat_history
        .iter()
        .rev()
        .find(|m| m.role == "error")
        .map(|m| m.content.clone());
    let agent_online = app.last_error.is_none();
    let ollama_online = app
        .state
        .as_ref()
        .map(|s| s.llm.ollama_online)
        .unwrap_or(false);
    let input_active = app.input_mode || !app.chat_input.trim().is_empty();

    let color_for = |name: &str, fallback: Color| -> Color {
        if flow_cfg.colorize {
            color_from_name(name).unwrap_or(fallback)
        } else {
            palette.main_text_color
        }
    };

    let input_mark = if input_active {
        styled_mark(
            "[ACT]",
            color_for(&flow_cfg.active_color, Color::LightYellow),
        )
    } else {
        styled_mark("[OK ]", color_for(&flow_cfg.ok_color, Color::LightGreen))
    };
    let request_mark = if pending {
        styled_mark("[RUN]", color_for(&flow_cfg.run_color, Color::Yellow))
    } else if chat_error
        .as_ref()
        .is_some_and(|e| e.contains("chat") || e.contains("decode"))
    {
        styled_mark("[ERR]", color_for(&flow_cfg.err_color, Color::LightRed))
    } else {
        styled_mark("[OK ]", color_for(&flow_cfg.ok_color, Color::LightGreen))
    };
    let agent_mark = if agent_online {
        styled_mark("[OK ]", color_for(&flow_cfg.ok_color, Color::LightGreen))
    } else {
        styled_mark("[ERR]", color_for(&flow_cfg.err_color, Color::LightRed))
    };
    let ollama_mark = if pending && !ollama_online {
        styled_mark("[RUN]", color_for(&flow_cfg.run_color, Color::Yellow))
    } else if ollama_online {
        styled_mark("[OK ]", color_for(&flow_cfg.ok_color, Color::LightGreen))
    } else {
        styled_mark("[ERR]", color_for(&flow_cfg.err_color, Color::LightRed))
    };
    let response_mark = if pending {
        styled_mark("[WAIT]", color_for(&flow_cfg.wait_color, Color::Yellow))
    } else if chat_error.is_some() {
        styled_mark("[ERR ]", color_for(&flow_cfg.err_color, Color::LightRed))
    } else {
        styled_mark("[OK  ]", color_for(&flow_cfg.ok_color, Color::LightGreen))
    };
    let render_mark = if pending {
        styled_mark("[WAIT]", color_for(&flow_cfg.wait_color, Color::Yellow))
    } else {
        styled_mark("[OK  ]", color_for(&flow_cfg.ok_color, Color::LightGreen))
    };

    let status_tail = if pending {
        "status: waiting response".to_string()
    } else if chat_error.is_some() {
        "status: chat error".to_string()
    } else if app.last_error.is_some() {
        "status: agent offline".to_string()
    } else if !ollama_online {
        "status: ollama offline".to_string()
    } else {
        "status: healthy".to_string()
    };

    vec![
        Line::from(vec![input_mark, Span::raw(" Input".to_string())]),
        Line::raw("  |".to_string()),
        Line::from(vec![request_mark, Span::raw(" POST /chat".to_string())]),
        Line::raw("  |".to_string()),
        Line::from(vec![agent_mark, Span::raw(" mini-agent".to_string())]),
        Line::raw("  |".to_string()),
        Line::from(vec![ollama_mark, Span::raw(" Ollama".to_string())]),
        Line::raw("  |".to_string()),
        Line::from(vec![response_mark, Span::raw(" Response".to_string())]),
        Line::raw("  |".to_string()),
        Line::from(vec![render_mark, Span::raw(" Render".to_string())]),
        Line::raw("".to_string()),
        Line::raw("[ACT] typing  [RUN] in-flight".to_string()),
        Line::raw("[WAIT] queued [OK] done [ERR] fail".to_string()),
        Line::raw(status_tail),
    ]
}

fn styled_mark(text: &'static str, color: Color) -> Span<'static> {
    Span::styled(
        text.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn role_display(app: &App, role: &str) -> (String, Style) {
    let palette = active_theme_palette(app);
    let style = Style::default().fg(palette.main_text_color);
    match role {
        "you" => (
            app.config.dashboard.user_name.clone(),
            Style::default()
                .fg(color_from_name(&app.config.dashboard.user_color).unwrap_or(palette.main_text_color)),
        ),
        "assistant" => (
            app.config.dashboard.assistant_name.clone(),
            Style::default().fg(
                color_from_name(&app.config.dashboard.assistant_color)
                    .unwrap_or(palette.main_text_color),
            ),
        ),
        "system" => ("system".to_string(), style),
        "error" => ("error".to_string(), style),
        other => (other.to_string(), style),
    }
}
