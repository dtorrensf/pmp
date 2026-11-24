// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Behaviour of the task list as a whole: rendering, keys, filters,
//! scrolling and dependency markers.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

use crate::db::stub::StubRepository;
use crate::screens::UiContext;

use super::*;

fn ui() -> UiContext {
    UiContext::default()
}

fn task(name: &str, status: Status) -> Task {
    Task {
        id: None,
        project_id: 1,
        version_id: 1,
        name: name.to_string(),
        description: String::new(),
        status,
        priority: Priority::Low,
        created_at: String::new(),
        completed_at: None,
    }
}

fn render_view(view: &TaskListView, width: u16, height: u16) -> String {
    use crate::screens::test_utils::buffer_to_string;
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| view.render(frame, frame.area(), &ui()))
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

fn numbered_tasks(count: usize) -> Vec<Task> {
    (0..count)
        .map(|i| task(&format!("Task{i:02}"), Status::Todo))
        .collect()
}

#[test]
fn selected_row_stays_visible_when_list_overflows() {
    let repo = StubRepository::default();
    let mut view = TaskListView::new();
    view.set_tasks(numbered_tasks(30));

    // Height 11 => 10 data rows (the header row takes 1).
    let before = render_view(&view, 40, 11);
    assert!(before.contains("Task00"));
    assert!(!before.contains("Task10"));

    for _ in 0..12 {
        view.handle_key(KeyCode::Down, &repo);
    }
    let after = render_view(&view, 40, 11);
    assert_eq!(view.selected(), 12);
    assert!(after.contains("Task12"), "selection not visible:\n{after}");
    assert!(
        !after.contains("Task00"),
        "top row should have scrolled out:\n{after}"
    );
}

#[test]
fn jumping_to_last_task_renders_the_tail_of_the_list() {
    let repo = StubRepository::default();
    let mut view = TaskListView::new();
    view.set_tasks(numbered_tasks(30));
    view.set_selected(29);
    view.handle_key(KeyCode::Down, &repo); // clamped at last task
    let rendered = render_view(&view, 40, 13);
    assert!(rendered.contains("Task29"));
    assert!(!rendered.contains("Task09"));
}

#[test]
fn moving_up_inside_view_keeps_rows_in_place() {
    let repo = StubRepository::default();
    let mut view = TaskListView::new();
    view.set_tasks(numbered_tasks(30));
    for _ in 0..15 {
        view.handle_key(KeyCode::Down, &repo);
    }
    let before = render_view(&view, 40, 13);
    view.handle_key(KeyCode::Up, &repo);
    let after = render_view(&view, 40, 13);
    assert!(before.contains("Task06") && after.contains("Task06"));
    assert!(after.contains("Task14"));
}

#[test]
fn short_list_renders_without_scrolling() {
    let mut view = TaskListView::new();
    view.set_tasks(numbered_tasks(3));
    let rendered = render_view(&view, 40, 13);
    assert!(rendered.contains("Task00") && rendered.contains("Task02"));
}

#[test]
fn long_names_are_cut_with_an_ellipsis_to_the_name_column() {
    let mut view = TaskListView::new();
    view.set_tasks(vec![task(
        "Una descripción de tarea bastante más larga que la columna",
        Status::Todo,
    )]);

    let rendered = render_view(&view, 40, 6);

    assert!(rendered.contains('…'), "{rendered}");
    assert!(!rendered.contains("columna"), "{rendered}");
}

#[test]
fn truncate_display_measures_columns_not_chars() {
    // Each CJK character is two columns wide.
    assert_eq!(truncate_display("ab漢字", 4), "ab…");
    assert_eq!(truncate_display("abc", 3), "abc");
    assert_eq!(truncate_display("abcd", 0), "");
}

#[test]
fn dependency_count_is_shown_next_to_the_name() {
    let mut with_deps = task("Alpha", Status::Todo);
    with_deps.id = Some(7);
    let mut view = TaskListView::new();
    view.set_tasks(vec![with_deps, task("Beta", Status::Todo)]);
    view.set_dependency_summaries(HashMap::from([(
        7,
        DependencySummary {
            total: 2,
            pending: 2,
        },
    )]));

    let rendered = render_view(&view, 60, 8);

    assert!(rendered.contains("↳ 2"), "{rendered}");
    assert_eq!(rendered.matches("↳").count(), 1, "{rendered}");
}

/// Foreground color of the dependency marker in a view rendered at 60x8.
fn dependency_marker_color(summary: DependencySummary) -> Option<ratatui::style::Color> {
    let mut with_deps = task("Alpha", Status::Todo);
    with_deps.id = Some(7);
    let mut view = TaskListView::new();
    view.set_tasks(vec![with_deps]);
    view.set_dependency_summaries(HashMap::from([(7, summary)]));

    let backend = TestBackend::new(60, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| view.render(frame, frame.area(), &ui()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    buffer
        .content()
        .iter()
        .find(|cell| cell.symbol() == "↳")
        .map(|cell| cell.fg)
}

#[test]
fn dependency_marker_is_green_when_all_dependencies_are_done() {
    let color = dependency_marker_color(DependencySummary {
        total: 2,
        pending: 0,
    });

    assert_eq!(color, Some(ui().success_style().fg.unwrap()));
}

#[test]
fn dependency_marker_is_yellow_while_a_dependency_is_pending() {
    let color = dependency_marker_color(DependencySummary {
        total: 2,
        pending: 1,
    });

    assert_eq!(color, Some(ui().warning_style().fg.unwrap()));
}

#[test]
fn selected_row_is_marked_with_the_selection_glyph() {
    let mut view = TaskListView::new();
    view.set_tasks(vec![
        task("Alpha", Status::Todo),
        task("Beta", Status::Todo),
    ]);

    let rendered = render_view(&view, 60, 8);

    assert!(rendered.contains("▸ Alpha"), "{rendered}");
    assert!(!rendered.contains("▸ Beta"), "{rendered}");
}

#[test]
fn rows_show_task_id_column() {
    let mut with_id = task("Alpha", Status::Todo);
    with_id.id = Some(42);
    let mut view = TaskListView::new();
    view.set_tasks(vec![with_id, task("Beta", Status::Done)]);

    let rendered = render_view(&view, 60, 10);
    assert!(rendered.contains("ID"), "missing ID header:\n{rendered}");
    assert!(rendered.contains("42"), "missing task id:\n{rendered}");
    assert!(rendered.contains("Beta"));
}

#[test]
fn status_and_priority_cells_carry_their_glyph() {
    let mut view = TaskListView::new();
    view.set_tasks(vec![task("Alpha", Status::InProgress)]);

    let rendered = render_view(&view, 60, 10);

    // The state is not communicated by color alone. Without a frame time
    // the spinner shows its first frame.
    assert!(rendered.contains("⠋ In Progress"), "{rendered}");
}

#[test]
fn header_row_uses_the_theme_instead_of_hardcoded_white() {
    let mut view = TaskListView::new();
    view.set_tasks(vec![task("Alpha", Status::Todo)]);

    let backend = TestBackend::new(60, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| view.render(frame, frame.area(), &ui()))
        .unwrap();
    let buffer = terminal.backend().buffer();

    // The "ID" header cell sits at the first column of the header row.
    let header_cell = &buffer[(1, 0)];
    assert_eq!(header_cell.fg, ui().theme().text);
    // White is invisible on light terminals.
    assert_ne!(header_cell.fg, ratatui::style::Color::White);
    assert!(header_cell.modifier.contains(Modifier::BOLD));
}

#[test]
fn all_tasks_ignores_active_filter() {
    let repo = StubRepository::default();
    let mut view = TaskListView::new();
    view.set_tasks(vec![task("A", Status::Todo), task("B", Status::Done)]);

    // Apply a status filter (Status: Todo) through the filter popup.
    view.handle_key(KeyCode::Char('f'), &repo);
    view.handle_key(KeyCode::Right, &repo);
    view.handle_key(KeyCode::Enter, &repo);

    assert_eq!(view.tasks().len(), 1);
    assert_eq!(view.all_tasks().len(), 2);
}

#[test]
fn filters_tasks_by_title_case_insensitively_and_partially() {
    let repo = StubRepository::default();
    let mut view = TaskListView::new();
    view.set_tasks(vec![
        task("Fix Login Flow", Status::Todo),
        task("Update README", Status::Todo),
        task("Login metrics", Status::Done),
    ]);

    view.handle_key(KeyCode::Char('f'), &repo);
    view.handle_key(KeyCode::Tab, &repo);
    view.handle_key(KeyCode::Tab, &repo);
    for character in "login".chars() {
        view.handle_key(KeyCode::Char(character), &repo);
    }
    view.handle_key(KeyCode::Enter, &repo);

    assert_eq!(view.tasks().len(), 2);
    assert!(
        view.tasks()
            .iter()
            .all(|task| task.name.to_lowercase().contains("login"))
    );
}

#[test]
fn title_filter_combines_with_status_and_empty_title_clears_it() {
    let repo = StubRepository::default();
    let mut view = TaskListView::new();
    view.set_tasks(vec![
        task("Login todo", Status::Todo),
        task("Login done", Status::Done),
        task("Other todo", Status::Todo),
    ]);

    view.handle_key(KeyCode::Char('f'), &repo);
    view.handle_key(KeyCode::Right, &repo);
    view.handle_key(KeyCode::Tab, &repo);
    view.handle_key(KeyCode::Tab, &repo);
    for character in "login".chars() {
        view.handle_key(KeyCode::Char(character), &repo);
    }
    view.handle_key(KeyCode::Enter, &repo);

    assert_eq!(view.tasks().len(), 1);
    assert_eq!(view.tasks()[0].name, "Login todo");

    view.handle_key(KeyCode::Char('f'), &repo);
    view.handle_key(KeyCode::Tab, &repo);
    view.handle_key(KeyCode::Tab, &repo);
    for _ in 0..5 {
        view.handle_key(KeyCode::Backspace, &repo);
    }
    view.handle_key(KeyCode::Enter, &repo);

    assert_eq!(view.filter_title(), None);
    assert_eq!(view.tasks().len(), 2);
}

fn caption(title: Option<&str>, status: Option<Status>, priority: Option<Priority>) -> String {
    super::table::filter_chips(title, status, priority, None, &ui())
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn filter_chips_show_each_active_filter() {
    assert_eq!(
        caption(None, Some(Status::Todo), None),
        "Tasks [Status: Todo]  x clear"
    );
    assert_eq!(
        caption(None, None, Some(Priority::High)),
        "Tasks [Priority: High]  x clear"
    );
    assert_eq!(
        caption(Some("login"), Some(Status::Done), Some(Priority::Low)),
        "Tasks [Title: login] [Status: Done] [Priority: Low]  x clear"
    );
}

#[test]
fn filter_chips_default_to_plain_title() {
    assert_eq!(caption(None, None, None), "Tasks");
}

#[test]
fn x_clears_every_active_filter() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    view.handle_key(KeyCode::Char('f'), &repo);
    view.handle_key(KeyCode::Right, &repo);
    view.handle_key(KeyCode::Enter, &repo);
    assert_eq!(view.filter_status(), Some(Status::Todo));

    view.handle_key(KeyCode::Char('x'), &repo);

    assert_eq!(view.filter_status(), None);
    assert_eq!(view.filter_priority(), None);
    assert_eq!(view.filter_title(), None);
}

#[test]
fn set_tasks_preserves_mode() {
    let repo = StubRepository::default();
    let mut view = TaskListView::new();
    view.set_tasks(vec![
        task("A", Status::Todo),
        task("B", Status::Done),
        task("C", Status::Todo),
    ]);

    // Apply a status filter through the filter popup.
    view.handle_key(KeyCode::Char('f'), &repo);
    view.handle_key(KeyCode::Right, &repo); // status -> Todo
    view.handle_key(KeyCode::Enter, &repo); // apply
    assert_eq!(view.tasks().len(), 2);

    // Re-enter filtering mode to verify it survives a data reload.
    view.handle_key(KeyCode::Char('f'), &repo);
    assert!(matches!(view.mode(), TaskListMode::Filtering(_)));

    // Reload data while the filter dialog is open.
    view.set_tasks(vec![
        task("D", Status::Todo),
        task("E", Status::Done),
        task("F", Status::Todo),
        task("G", Status::Todo),
    ]);

    // Mode, reapplied filter, and clamped selection must survive.
    assert!(matches!(view.mode(), TaskListMode::Filtering(_)));
    assert_eq!(view.tasks().len(), 3);
    assert!(view.tasks().iter().all(|t| t.status == Status::Todo));
    assert!(view.selected() < view.tasks().len());
}

fn task_with_id(id: Option<i64>, name: &str, status: Status, priority: Priority) -> Task {
    Task {
        id,
        priority,
        ..task(name, status)
    }
}

fn ids(view: &TaskListView) -> Vec<Option<i64>> {
    view.tasks().iter().map(|task| task.id).collect()
}

/// Five tasks in repository order (priority, as the repo returns them),
/// one of them without id.
fn sortable_view() -> TaskListView {
    let mut view = TaskListView::new();
    view.set_tasks(vec![
        task_with_id(Some(12), "Twelve", Status::Done, Priority::High),
        task_with_id(None, "Unsaved", Status::Todo, Priority::High),
        task_with_id(Some(3), "Three", Status::Todo, Priority::Low),
        task_with_id(Some(7), "Seven", Status::InProgress, Priority::Medium),
        task_with_id(Some(5), "Five", Status::Todo, Priority::Low),
    ]);
    view
}

#[test]
fn sort_mode_cycles_priority_then_id_asc_then_id_desc() {
    let repo = StubRepository::default();
    let mut view = TaskListView::new();
    assert_eq!(view.sort_mode(), SortMode::Priority);

    view.handle_key(KeyCode::Char('o'), &repo);
    assert_eq!(view.sort_mode(), SortMode::IdAsc);
    view.handle_key(KeyCode::Char('o'), &repo);
    assert_eq!(view.sort_mode(), SortMode::IdDesc);
    view.handle_key(KeyCode::Char('o'), &repo);
    assert_eq!(view.sort_mode(), SortMode::Priority);
}

#[test]
fn default_priority_mode_keeps_repository_order() {
    let view = sortable_view();
    assert_eq!(ids(&view), vec![Some(12), None, Some(3), Some(7), Some(5)]);
}

#[test]
fn id_asc_orders_by_id_and_puts_tasks_without_id_last() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    view.handle_key(KeyCode::Char('o'), &repo);

    assert_eq!(ids(&view), vec![Some(3), Some(5), Some(7), Some(12), None]);
}

#[test]
fn id_desc_orders_by_id_descending_and_still_puts_missing_ids_last() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    view.handle_key(KeyCode::Char('o'), &repo);
    view.handle_key(KeyCode::Char('o'), &repo);

    assert_eq!(ids(&view), vec![Some(12), Some(7), Some(5), Some(3), None]);
}

#[test]
fn id_sort_does_not_move_done_tasks_to_the_end() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    view.handle_key(KeyCode::Char('o'), &repo);

    // Twelve is Done but keeps its place by id, not at the bottom.
    assert_eq!(view.tasks()[3].name, "Twelve");
}

#[test]
fn id_sort_combines_with_status_filter() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    view.handle_key(KeyCode::Char('o'), &repo);

    view.handle_key(KeyCode::Char('f'), &repo);
    view.handle_key(KeyCode::Right, &repo); // status -> Todo
    view.handle_key(KeyCode::Enter, &repo);

    assert_eq!(ids(&view), vec![Some(3), Some(5), None]);
}

#[test]
fn id_sort_combines_with_title_filter() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    view.handle_key(KeyCode::Char('o'), &repo);
    view.handle_key(KeyCode::Char('o'), &repo);

    view.handle_key(KeyCode::Char('f'), &repo);
    view.handle_key(KeyCode::Tab, &repo);
    view.handle_key(KeyCode::Tab, &repo);
    for character in "e".chars() {
        view.handle_key(KeyCode::Char(character), &repo);
    }
    view.handle_key(KeyCode::Enter, &repo);

    // Every name contains "e"; the list is still ordered by id descending.
    assert_eq!(ids(&view), vec![Some(12), Some(7), Some(5), Some(3), None]);
    view.handle_key(KeyCode::Char('o'), &repo); // back to priority order
    assert_eq!(ids(&view), vec![Some(12), None, Some(3), Some(7), Some(5)]);
}

#[test]
fn cycling_sort_keeps_the_same_task_selected() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    // Select "Seven", which is at index 3 in priority order.
    view.set_selected(3);
    assert_eq!(view.tasks()[view.selected()].id, Some(7));

    view.handle_key(KeyCode::Char('o'), &repo); // IdAsc: Seven is at index 2
    assert_eq!(view.tasks()[view.selected()].id, Some(7));
    assert_eq!(view.selected(), 2);

    view.handle_key(KeyCode::Char('o'), &repo); // IdDesc: Seven is at index 1
    assert_eq!(view.tasks()[view.selected()].id, Some(7));
    assert_eq!(view.selected(), 1);
}

#[test]
fn sort_key_is_ignored_while_filter_popup_is_open() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    view.handle_key(KeyCode::Char('f'), &repo);
    view.handle_key(KeyCode::Tab, &repo);
    view.handle_key(KeyCode::Tab, &repo);

    view.handle_key(KeyCode::Char('o'), &repo);

    assert_eq!(view.sort_mode(), SortMode::Priority);
    match view.mode() {
        TaskListMode::Filtering(state) => assert_eq!(state.filter_title, "o"),
        other => panic!("expected filter popup, got {other:?}"),
    }
}

#[test]
fn sort_mode_survives_data_reload() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    view.handle_key(KeyCode::Char('o'), &repo);

    view.set_tasks(vec![
        task_with_id(Some(9), "Nine", Status::Todo, Priority::Low),
        task_with_id(Some(1), "One", Status::Todo, Priority::Low),
    ]);

    assert_eq!(view.sort_mode(), SortMode::IdAsc);
    assert_eq!(ids(&view), vec![Some(1), Some(9)]);
}

fn caption_of(view: &TaskListView) -> String {
    view.table_title(&ui())
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn table_title_shows_the_active_id_direction() {
    let repo = StubRepository::default();
    let mut view = sortable_view();
    assert!(!caption_of(&view).contains("ID ↑"));

    view.handle_key(KeyCode::Char('o'), &repo);
    assert_eq!(caption_of(&view), "Tasks · ID ↑");

    view.handle_key(KeyCode::Char('o'), &repo);
    assert_eq!(caption_of(&view), "Tasks · ID ↓");
}

mod error_propagation {
    use super::*;
    use crate::screens::test_utils::FaultyRepository;

    /// The delete confirmation needs the number of dependent tasks. If that
    /// query fails, the dialog must not open: confirming would hide the impact
    /// of a destructive action.
    #[test]
    fn delete_reports_the_error_and_opens_no_dialog() {
        let repo = FaultyRepository {
            fail_dependents: true,
            ..FaultyRepository::default()
        };
        let mut with_id = task("Alpha", Status::Todo);
        with_id.id = Some(7);
        let mut view = TaskListView::new();
        view.set_tasks(vec![with_id]);

        let action = view.handle_key(KeyCode::Char('d'), &repo);

        assert!(
            matches!(action, Some(Action::ReportError { .. })),
            "expected an error action, got {action:?}"
        );
        assert!(
            view.is_browsing(),
            "no confirmation may open without the dependency impact"
        );
        assert_ne!(action, Some(Action::DeleteTask { task_id: 7 }));
    }

    /// With a working query the dialog still opens with the impact message.
    #[test]
    fn delete_opens_the_dialog_when_the_query_works() {
        use crate::db::repository::{ProjectRepository, RepositoryPolicy, TaskRepository};
        let repo = StubRepository::default();
        let project_id = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let stored = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "Alpha",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let mut view = TaskListView::new();
        view.set_tasks(vec![stored]);

        let action = view.handle_key(KeyCode::Char('d'), &repo);

        assert_eq!(action, None);
        assert!(matches!(view.mode(), TaskListMode::ConfirmDelete(_)));
    }
}
