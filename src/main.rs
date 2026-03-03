use std::collections::HashSet;
use std::fs;
use std::io::{self, Stdout, Write, stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::{Color, Print, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};

const K_QUIT: char = 'q';
const K_SAVE: char = 's';
const K_SAVE_AS: char = 'a';
const K_OPEN_FILE: char = 'o';
const K_OPEN_FOLDER: char = 'g';
const K_CLOSE_FOLDER: char = 'x';
const K_NEW_FILE: char = 'n';
const K_NEW_FOLDER: char = 'm';
const K_CLOSE_BUFFER: char = 'w';
const K_TOGGLE_FOCUS: char = 'b';

#[derive(Clone)]
struct Buffer {
    name: String,
    path: Option<PathBuf>,
    lines: Vec<String>,
    cursor_x: usize,
    cursor_y: usize,
    scroll_y: usize,
    dirty: bool,
}

impl Buffer {
    fn empty(name: String) -> Self {
        Self {
            name,
            path: None,
            lines: vec![String::new()],
            cursor_x: 0,
            cursor_y: 0,
            scroll_y: 0,
            dirty: false,
        }
    }

    fn from_file(path: PathBuf) -> io::Result<Self> {
        let contents = fs::read_to_string(&path)?;
        let mut lines: Vec<String> = contents.lines().map(ToOwned::to_owned).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        Ok(Self {
            name,
            path: Some(path),
            lines,
            cursor_x: 0,
            cursor_y: 0,
            scroll_y: 0,
            dirty: false,
        })
    }

    fn line_len(&self, y: usize) -> usize {
        self.lines.get(y).map(|l| l.len()).unwrap_or(0)
    }

    fn insert_char(&mut self, c: char) {
        if let Some(line) = self.lines.get_mut(self.cursor_y) {
            let at = self.cursor_x.min(line.len());
            line.insert(at, c);
            self.cursor_x = at + c.len_utf8();
            self.dirty = true;
        }
    }

    fn new_line(&mut self) {
        if self.cursor_y >= self.lines.len() {
            self.lines.push(String::new());
            self.cursor_y = self.lines.len().saturating_sub(1);
            self.cursor_x = 0;
            return;
        }

        let split_at = self.cursor_x.min(self.lines[self.cursor_y].len());
        let rest = self.lines[self.cursor_y].split_off(split_at);
        self.cursor_y += 1;
        self.cursor_x = 0;
        self.lines.insert(self.cursor_y, rest);
        self.dirty = true;
    }

    fn backspace(&mut self) {
        if self.cursor_y >= self.lines.len() {
            return;
        }

        if self.cursor_x > 0 {
            let line = &mut self.lines[self.cursor_y];
            if self.cursor_x <= line.len() {
                line.remove(self.cursor_x - 1);
                self.cursor_x -= 1;
                self.dirty = true;
            }
        } else if self.cursor_y > 0 {
            let current = self.lines.remove(self.cursor_y);
            self.cursor_y -= 1;
            self.cursor_x = self.lines[self.cursor_y].len();
            self.lines[self.cursor_y].push_str(&current);
            self.dirty = true;
        }
    }

    fn move_left(&mut self) {
        if self.cursor_x > 0 {
            self.cursor_x -= 1;
        } else if self.cursor_y > 0 {
            self.cursor_y -= 1;
            self.cursor_x = self.line_len(self.cursor_y);
        }
    }

    fn move_right(&mut self) {
        let line_len = self.line_len(self.cursor_y);
        if self.cursor_x < line_len {
            self.cursor_x += 1;
        } else if self.cursor_y + 1 < self.lines.len() {
            self.cursor_y += 1;
            self.cursor_x = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor_y > 0 {
            self.cursor_y -= 1;
            self.cursor_x = self.cursor_x.min(self.line_len(self.cursor_y));
        }
    }

    fn move_down(&mut self) {
        if self.cursor_y + 1 < self.lines.len() {
            self.cursor_y += 1;
            self.cursor_x = self.cursor_x.min(self.line_len(self.cursor_y));
        }
    }

    fn save(&mut self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Buffer has no target path",
            ));
        };
        fs::write(path, self.lines.join("\n"))?;
        self.dirty = false;
        Ok(())
    }

    fn save_as(&mut self, path: PathBuf) -> io::Result<()> {
        fs::write(&path, self.lines.join("\n"))?;
        self.path = Some(path.clone());
        self.name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        self.dirty = false;
        Ok(())
    }
}

#[derive(Clone)]
struct ExplorerItem {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    name: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Explorer,
    Editor,
}

#[derive(Clone)]
enum PromptKind {
    OpenFile,
    OpenFolder,
    NewFile,
    NewFolder,
    SaveAs,
}

impl PromptKind {
    fn title(&self) -> &'static str {
        match self {
            Self::OpenFile => "Open file",
            Self::OpenFolder => "Open folder",
            Self::NewFile => "Create file",
            Self::NewFolder => "Create folder",
            Self::SaveAs => "Save as",
        }
    }
}

#[derive(Clone)]
enum Mode {
    Normal,
    Prompt { kind: PromptKind, input: String },
}

struct App {
    workspace_root: Option<PathBuf>,
    expanded_dirs: HashSet<PathBuf>,
    explorer: Vec<ExplorerItem>,
    explorer_index: usize,
    explorer_scroll: usize,
    buffers: Vec<Buffer>,
    active_buffer: usize,
    untitled_counter: usize,
    focus: FocusPane,
    mode: Mode,
    status: String,
    should_quit: bool,
}

impl App {
    fn new(arg: Option<PathBuf>) -> Self {
        let mut app = Self {
            workspace_root: None,
            expanded_dirs: HashSet::new(),
            explorer: Vec::new(),
            explorer_index: 0,
            explorer_scroll: 0,
            buffers: vec![Buffer::empty(String::from("untitled-1"))],
            active_buffer: 0,
            untitled_counter: 1,
            focus: FocusPane::Editor,
            mode: Mode::Normal,
            status: String::from("Ready"),
            should_quit: false,
        };

        if let Some(path) = arg {
            if path.is_dir() {
                if let Err(err) = app.open_workspace(path) {
                    app.status = format!("Workspace error: {}", err);
                }
            } else {
                let parent = path.parent().map(Path::to_path_buf);
                if let Some(parent_dir) = parent {
                    let _ = app.open_workspace(parent_dir);
                }
                if let Err(err) = app.open_buffer(path) {
                    app.status = format!("Open file error: {}", err);
                }
            }
        }

        app
    }

    fn active_buffer(&self) -> &Buffer {
        &self.buffers[self.active_buffer]
    }

    fn active_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.active_buffer]
    }

    fn next_untitled_name(&mut self) -> String {
        self.untitled_counter += 1;
        format!("untitled-{}", self.untitled_counter)
    }

    fn resolve_path(&self, input: &str) -> PathBuf {
        let trimmed = input.trim();
        let expanded = if trimmed == "~" {
            std::env::var("HOME").unwrap_or_else(|_| String::from("~"))
        } else if let Some(rest) = trimmed.strip_prefix("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| String::from(""));
            if home.is_empty() {
                trimmed.to_string()
            } else {
                format!("{}/{}", home, rest)
            }
        } else {
            trimmed.to_string()
        };

        let path = PathBuf::from(expanded);
        if path.is_absolute() {
            path
        } else if let Some(root) = &self.workspace_root {
            root.join(path)
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    }

    fn open_workspace(&mut self, path: PathBuf) -> io::Result<()> {
        let root = fs::canonicalize(path)?;
        self.workspace_root = Some(root.clone());
        self.expanded_dirs.clear();
        self.expanded_dirs.insert(root.clone());
        self.refresh_explorer()?;
        self.status = format!("Opened folder {}", root.display());
        Ok(())
    }

    fn close_workspace(&mut self) {
        self.workspace_root = None;
        self.expanded_dirs.clear();
        self.explorer.clear();
        self.explorer_index = 0;
        self.explorer_scroll = 0;
        self.status = String::from("Closed folder");
    }

    fn refresh_explorer(&mut self) -> io::Result<()> {
        self.explorer.clear();
        if let Some(root) = self.workspace_root.clone() {
            self.push_tree_items(&root, 0)?;
            if self.explorer_index >= self.explorer.len() {
                self.explorer_index = self.explorer.len().saturating_sub(1);
            }
        }
        Ok(())
    }

    fn push_tree_items(&mut self, path: &Path, depth: usize) -> io::Result<()> {
        let name = if depth == 0 {
            path.display().to_string()
        } else {
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string())
        };

        let is_dir = path.is_dir();
        self.explorer.push(ExplorerItem {
            path: path.to_path_buf(),
            depth,
            is_dir,
            name,
        });

        if is_dir && self.expanded_dirs.contains(path) {
            let mut entries: Vec<PathBuf> = fs::read_dir(path)?
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .collect();

            entries.sort_by(|a, b| {
                let a_dir = a.is_dir();
                let b_dir = b.is_dir();
                match (a_dir, b_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                        .cmp(
                            &b.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_lowercase(),
                        ),
                }
            });

            for child in entries {
                let _ = self.push_tree_items(&child, depth + 1);
            }
        }
        Ok(())
    }

    fn selected_item(&self) -> Option<&ExplorerItem> {
        self.explorer.get(self.explorer_index)
    }

    fn toggle_selected_dir(&mut self) {
        let Some(item) = self.selected_item().cloned() else {
            return;
        };
        if !item.is_dir {
            return;
        }

        if self.expanded_dirs.contains(&item.path) {
            self.expanded_dirs.remove(&item.path);
            self.status = format!("Collapsed {}", item.path.display());
        } else {
            self.expanded_dirs.insert(item.path.clone());
            self.status = format!("Expanded {}", item.path.display());
        }
        if let Err(err) = self.refresh_explorer() {
            self.status = format!("Explorer refresh error: {}", err);
        }
    }

    fn move_explorer_selection(&mut self, delta: isize) {
        if self.explorer.is_empty() {
            return;
        }
        let idx = self.explorer_index as isize + delta;
        self.explorer_index = idx.clamp(0, (self.explorer.len() - 1) as isize) as usize;
    }

    fn open_selected_item(&mut self) {
        let Some(item) = self.selected_item().cloned() else {
            return;
        };

        if item.is_dir {
            self.toggle_selected_dir();
        } else if let Err(err) = self.open_buffer(item.path.clone()) {
            self.status = format!("Open file error: {}", err);
        }
    }

    fn collapse_selected_or_parent(&mut self) {
        let Some(item) = self.selected_item().cloned() else {
            return;
        };

        if item.is_dir && self.expanded_dirs.contains(&item.path) {
            self.expanded_dirs.remove(&item.path);
            let _ = self.refresh_explorer();
            return;
        }

        if let Some(parent) = item.path.parent() {
            let parent_path = parent.to_path_buf();
            self.expanded_dirs.remove(&parent_path);
            if let Err(err) = self.refresh_explorer() {
                self.status = format!("Explorer refresh error: {}", err);
                return;
            }
            if let Some(pos) = self.explorer.iter().position(|e| e.path == parent_path) {
                self.explorer_index = pos;
            }
        }
    }

    fn expand_selected(&mut self) {
        let Some(item) = self.selected_item().cloned() else {
            return;
        };

        if item.is_dir {
            self.expanded_dirs.insert(item.path);
            let _ = self.refresh_explorer();
        } else {
            self.open_selected_item();
        }
    }

    fn open_buffer(&mut self, path: PathBuf) -> io::Result<()> {
        let real_path = fs::canonicalize(path)?;
        if let Some((idx, _)) = self
            .buffers
            .iter()
            .enumerate()
            .find(|(_, b)| b.path.as_ref() == Some(&real_path))
        {
            self.active_buffer = idx;
            self.status = format!("Focused {}", real_path.display());
            return Ok(());
        }

        let buffer = Buffer::from_file(real_path.clone())?;
        let only_empty = self.buffers.len() == 1
            && self.buffers[0].path.is_none()
            && self.buffers[0].lines.len() == 1
            && self.buffers[0].lines[0].is_empty()
            && !self.buffers[0].dirty;

        if only_empty {
            self.buffers[0] = buffer;
            self.active_buffer = 0;
        } else {
            self.buffers.push(buffer);
            self.active_buffer = self.buffers.len() - 1;
        }

        self.status = format!("Opened {}", real_path.display());
        Ok(())
    }

    fn new_empty_buffer(&mut self) {
        let name = self.next_untitled_name();
        self.buffers.push(Buffer::empty(name.clone()));
        self.active_buffer = self.buffers.len() - 1;
        self.status = format!("Created {}", name);
    }

    fn close_active_buffer(&mut self) {
        if self.buffers.len() == 1 {
            self.buffers[0] = Buffer::empty(String::from("untitled-1"));
            self.active_buffer = 0;
            self.status = String::from("Reset to empty buffer");
            return;
        }
        self.buffers.remove(self.active_buffer);
        if self.active_buffer >= self.buffers.len() {
            self.active_buffer = self.buffers.len().saturating_sub(1);
        }
        self.status = String::from("Closed active buffer");
    }

    fn save_active(&mut self) {
        let had_path = self.active_buffer().path.is_some();
        if had_path {
            match self.active_buffer_mut().save() {
                Ok(_) => {
                    let name = self.active_buffer().name.clone();
                    self.status = format!("Saved {}", name);
                }
                Err(err) => self.status = format!("Save error: {}", err),
            }
        } else {
            self.mode = Mode::Prompt {
                kind: PromptKind::SaveAs,
                input: String::new(),
            };
            self.status = String::from("Save as path:");
        }
    }

    fn save_active_as(&mut self, path: PathBuf) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match self.active_buffer_mut().save_as(path.clone()) {
            Ok(_) => {
                self.status = format!("Saved {}", path.display());
                if self.workspace_root.is_some() {
                    let _ = self.refresh_explorer();
                }
            }
            Err(err) => self.status = format!("Save-as error: {}", err),
        }
    }

    fn start_prompt(&mut self, kind: PromptKind) {
        self.mode = Mode::Prompt {
            kind,
            input: String::new(),
        };
    }

    fn execute_prompt(&mut self, kind: PromptKind, input: String) {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            self.mode = Mode::Normal;
            self.status = String::from("Prompt canceled");
            return;
        }

        match kind {
            PromptKind::OpenFile => {
                let path = self.resolve_path(trimmed);
                match self.open_buffer(path) {
                    Ok(_) => self.focus = FocusPane::Editor,
                    Err(err) => self.status = format!("Open file error: {}", err),
                }
            }
            PromptKind::OpenFolder => {
                let path = self.resolve_path(trimmed);
                match self.open_workspace(path) {
                    Ok(_) => self.focus = FocusPane::Explorer,
                    Err(err) => self.status = format!("Open folder error: {}", err),
                }
            }
            PromptKind::NewFile => {
                let path = self.resolve_path(trimmed);
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                match fs::write(&path, "") {
                    Ok(_) => {
                        let _ = self.open_buffer(path.clone());
                        let _ = self.refresh_explorer();
                        self.status = format!("Created file {}", path.display());
                    }
                    Err(err) => self.status = format!("Create file error: {}", err),
                }
            }
            PromptKind::NewFolder => {
                let path = self.resolve_path(trimmed);
                match fs::create_dir_all(&path) {
                    Ok(_) => {
                        self.expanded_dirs.insert(path.clone());
                        let _ = self.refresh_explorer();
                        self.status = format!("Created folder {}", path.display());
                    }
                    Err(err) => self.status = format!("Create folder error: {}", err),
                }
            }
            PromptKind::SaveAs => {
                let path = self.resolve_path(trimmed);
                self.save_active_as(path);
            }
        }
        self.mode = Mode::Normal;
    }
}

fn main() -> io::Result<()> {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let mut app = App::new(arg);
    app.status = String::from(
        "Ctrl+Q quit | Ctrl+S save | Ctrl+O open file | Ctrl+G open folder | Ctrl+B toggle pane",
    );

    let mut out = stdout();
    enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, Hide)?;

    let run_result = run_editor(&mut out, &mut app);

    disable_raw_mode()?;
    execute!(out, Show, LeaveAlternateScreen)?;
    run_result
}

fn run_editor(out: &mut Stdout, app: &mut App) -> io::Result<()> {
    draw_intro(out)?;

    while !app.should_quit {
        draw_editor(out, app)?;
        if event::poll(Duration::from_millis(120))? {
            if let Event::Key(key) = event::read()? {
                handle_key_event(app, key);
            }
        }
    }

    Ok(())
}

fn handle_key_event(app: &mut App, key: KeyEvent) {
    match app.mode.clone() {
        Mode::Prompt { kind, mut input } => {
            match key.code {
                KeyCode::Esc => {
                    app.mode = Mode::Normal;
                    app.status = String::from("Prompt canceled");
                }
                KeyCode::Enter => {
                    app.execute_prompt(kind, input);
                }
                KeyCode::Backspace => {
                    input.pop();
                    app.mode = Mode::Prompt { kind, input };
                }
                KeyCode::Char(c) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        input.push(c);
                        app.mode = Mode::Prompt { kind, input };
                    }
                }
                _ => {}
            }
            return;
        }
        Mode::Normal => {}
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char(c) if c == K_QUIT => {
                app.should_quit = true;
                return;
            }
            KeyCode::Char(c) if c == K_SAVE => {
                app.save_active();
                return;
            }
            KeyCode::Char(c) if c == K_SAVE_AS => {
                app.start_prompt(PromptKind::SaveAs);
                return;
            }
            KeyCode::Char(c) if c == K_OPEN_FILE => {
                app.start_prompt(PromptKind::OpenFile);
                return;
            }
            KeyCode::Char(c) if c == K_OPEN_FOLDER => {
                app.start_prompt(PromptKind::OpenFolder);
                return;
            }
            KeyCode::Char(c) if c == K_CLOSE_FOLDER => {
                app.close_workspace();
                return;
            }
            KeyCode::Char(c) if c == K_NEW_FILE => {
                app.start_prompt(PromptKind::NewFile);
                return;
            }
            KeyCode::Char(c) if c == K_NEW_FOLDER => {
                app.start_prompt(PromptKind::NewFolder);
                return;
            }
            KeyCode::Char(c) if c == K_CLOSE_BUFFER => {
                app.close_active_buffer();
                return;
            }
            KeyCode::Char(c) if c == K_TOGGLE_FOCUS => {
                app.focus = if app.focus == FocusPane::Editor {
                    FocusPane::Explorer
                } else {
                    FocusPane::Editor
                };
                return;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::F(1) => {
            app.start_prompt(PromptKind::OpenFile);
            return;
        }
        KeyCode::F(2) => {
            app.start_prompt(PromptKind::OpenFolder);
            return;
        }
        KeyCode::F(3) => {
            app.start_prompt(PromptKind::NewFile);
            return;
        }
        KeyCode::F(4) => {
            app.start_prompt(PromptKind::NewFolder);
            return;
        }
        KeyCode::F(5) => {
            app.save_active();
            return;
        }
        KeyCode::F(6) => {
            app.start_prompt(PromptKind::SaveAs);
            return;
        }
        KeyCode::F(7) => {
            app.close_active_buffer();
            return;
        }
        KeyCode::F(8) => {
            app.close_workspace();
            return;
        }
        KeyCode::F(9) => {
            app.focus = if app.focus == FocusPane::Editor {
                FocusPane::Explorer
            } else {
                FocusPane::Editor
            };
            return;
        }
        KeyCode::F(10) => {
            app.should_quit = true;
            return;
        }
        _ => {}
    }

    match app.focus {
        FocusPane::Explorer => handle_explorer_keys(app, key),
        FocusPane::Editor => handle_editor_keys(app, key),
    }
}

fn handle_explorer_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up => app.move_explorer_selection(-1),
        KeyCode::Down => app.move_explorer_selection(1),
        KeyCode::Left => app.collapse_selected_or_parent(),
        KeyCode::Right => app.expand_selected(),
        KeyCode::Enter => app.open_selected_item(),
        KeyCode::Tab => app.focus = FocusPane::Editor,
        _ => {}
    }
}

fn handle_editor_keys(app: &mut App, key: KeyEvent) {
    let buffer = app.active_buffer_mut();
    match key.code {
        KeyCode::Left => buffer.move_left(),
        KeyCode::Right => buffer.move_right(),
        KeyCode::Up => buffer.move_up(),
        KeyCode::Down => buffer.move_down(),
        KeyCode::Enter => buffer.new_line(),
        KeyCode::Backspace => buffer.backspace(),
        KeyCode::Tab => {
            for _ in 0..4 {
                buffer.insert_char(' ');
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => buffer.insert_char(c),
        _ => {}
    }
}

fn draw_intro(out: &mut Stdout) -> io::Result<()> {
    let (w, h) = terminal::size()?;
    queue!(out, Clear(ClearType::All), MoveTo(0, 0))?;

    let msg = "hello draken -- let us cook";
    let x = ((w as usize).saturating_sub(msg.len()) / 2) as u16;
    let y = h.saturating_div(3);
    queue!(
        out,
        SetForegroundColor(Color::Yellow),
        MoveTo(x, y),
        Print(msg),
        SetForegroundColor(Color::Reset)
    )?;
    out.flush()?;
    std::thread::sleep(Duration::from_millis(650));

    for frame in dragon_frames() {
        queue!(out, Clear(ClearType::All))?;
        let lines: Vec<&str> = frame.lines().collect();
        let frame_h = lines.len() as u16;
        let top = h.saturating_sub(frame_h).saturating_div(2);

        for (i, line) in lines.iter().enumerate() {
            let px = ((w as usize).saturating_sub(line.len()) / 2) as u16;
            queue!(
                out,
                SetForegroundColor(Color::Green),
                MoveTo(px, top + i as u16),
                Print(*line),
                SetForegroundColor(Color::Reset)
            )?;
        }
        out.flush()?;
        std::thread::sleep(Duration::from_millis(220));
    }

    Ok(())
}

fn dragon_frames() -> [&'static str; 4] {
    [
        r#"
                 /\_/\
           ____ / o o \
         /~____  =o= /
        (______)__m_m)  ~~
"#,
        r#"
                 /\_/\
           ____ / o o \
         /~____  =o= /
        (______)__m_m)  ~~~~~
"#,
        r#"
                 /\_/\
           ____ / ^ ^ \
         /~____  =o= /
        (______)__m_m)  ~~~~~~~~~
"#,
        r#"
                 /\_/\
           ____ / o o \
         /~____  =o= /
        (______)__m_m)  ~~
"#,
    ]
}

fn draw_editor(out: &mut Stdout, app: &mut App) -> io::Result<()> {
    let (w, h) = terminal::size()?;
    if w < 30 || h < 12 {
        queue!(
            out,
            Clear(ClearType::All),
            MoveTo(1, 1),
            SetForegroundColor(Color::Red),
            Print("Terminal too small (need at least 30x12)"),
            SetForegroundColor(Color::Reset)
        )?;
        out.flush()?;
        return Ok(());
    }

    let border = Color::Rgb { r: 196, g: 168, b: 93 };
    let bg = Color::Rgb { r: 18, g: 24, b: 40 };
    let fg = Color::Rgb { r: 245, g: 248, b: 255 };
    let accent = Color::Rgb { r: 114, g: 205, b: 255 };
    let status_bg = Color::Rgb { r: 10, g: 50, b: 74 };
    let status_fg = Color::Rgb { r: 245, g: 255, b: 255 };

    for y in 0..h {
        queue!(
            out,
            MoveTo(0, y),
            SetBackgroundColor(border),
            Print(" ".repeat(w as usize))
        )?;
    }

    let inner_x = 1;
    let inner_y = 1;
    let inner_w = w.saturating_sub(2);
    let inner_h = h.saturating_sub(2);

    for y in inner_y..(inner_y + inner_h) {
        queue!(
            out,
            MoveTo(inner_x, y),
            SetBackgroundColor(bg),
            Print(" ".repeat(inner_w as usize))
        )?;
    }

    let explorer_w = inner_w.saturating_mul(28).saturating_div(100).max(20);
    let editor_x = inner_x + explorer_w + 1;
    let editor_w = inner_w.saturating_sub(explorer_w + 1);

    for y in inner_y..(inner_y + inner_h) {
        queue!(
            out,
            MoveTo(inner_x + explorer_w, y),
            SetBackgroundColor(border),
            Print(" ")
        )?;
    }

    draw_header(out, app, inner_x, inner_y, inner_w, fg, accent)?;
    draw_explorer(
        out,
        app,
        inner_x + 1,
        inner_y + 2,
        explorer_w.saturating_sub(2),
        inner_h.saturating_sub(5),
        fg,
        accent,
        bg,
    )?;
    let editor_cursor = draw_editor_pane(
        out,
        app,
        editor_x + 1,
        inner_y + 2,
        editor_w.saturating_sub(2),
        inner_h.saturating_sub(5),
        fg,
        accent,
    )?;

    draw_status_line(
        out,
        app,
        inner_x + 1,
        inner_y + inner_h.saturating_sub(2),
        inner_w.saturating_sub(2),
        status_bg,
        status_fg,
    )?;

    if let Mode::Prompt { kind, input } = &app.mode {
        let prompt = format!("{}: {}", kind.title(), input);
        let clipped: String = prompt.chars().take(inner_w.saturating_sub(4) as usize).collect();
        queue!(
            out,
            SetBackgroundColor(status_bg),
            SetForegroundColor(status_fg),
            MoveTo(inner_x + 2, inner_y + inner_h.saturating_sub(2)),
            Print(clipped)
        )?;

        let cursor_x = (inner_x + 2)
            .saturating_add(kind.title().len() as u16)
            .saturating_add(2)
            .saturating_add(input.len() as u16)
            .min(inner_x + inner_w.saturating_sub(2));
        queue!(out, MoveTo(cursor_x, inner_y + inner_h.saturating_sub(2)))?;
    } else if app.focus == FocusPane::Explorer {
        let y = inner_y + 2 + (app.explorer_index.saturating_sub(app.explorer_scroll) as u16);
        queue!(out, MoveTo(inner_x + 2, y))?;
    } else {
        queue!(out, MoveTo(editor_cursor.0, editor_cursor.1))?;
    }

    queue!(out, SetBackgroundColor(Color::Reset), SetForegroundColor(Color::Reset))?;
    out.flush()?;
    Ok(())
}

fn draw_header(
    out: &mut Stdout,
    app: &App,
    x: u16,
    y: u16,
    w: u16,
    fg: Color,
    accent: Color,
) -> io::Result<()> {
    let ws = app
        .workspace_root
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| String::from("[no folder opened]"));
    let text = format!(
        " TERMX | ws: {} | buffers: {} | focus: {} ",
        ws,
        app.buffers.len(),
        if app.focus == FocusPane::Editor {
            "editor"
        } else {
            "explorer"
        }
    );
    let clipped: String = text.chars().take(w.saturating_sub(2) as usize).collect();
    queue!(
        out,
        SetForegroundColor(accent),
        MoveTo(x + 1, y),
        Print(clipped),
        SetForegroundColor(fg)
    )?;
    Ok(())
}

fn draw_explorer(
    out: &mut Stdout,
    app: &mut App,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    fg: Color,
    accent: Color,
    bg: Color,
) -> io::Result<()> {
    if h == 0 || w == 0 {
        return Ok(());
    }

    if app.explorer_index < app.explorer_scroll {
        app.explorer_scroll = app.explorer_index;
    }
    if app.explorer_index >= app.explorer_scroll + h as usize {
        app.explorer_scroll = app.explorer_index + 1 - h as usize;
    }

    queue!(
        out,
        SetForegroundColor(accent),
        MoveTo(x, y.saturating_sub(1)),
        Print("Explorer"),
        SetForegroundColor(fg)
    )?;

    for row in 0..h as usize {
        let idx = app.explorer_scroll + row;
        let py = y + row as u16;
        queue!(out, MoveTo(x, py))?;

        if let Some(item) = app.explorer.get(idx) {
            let mut marker = " ";
            if item.is_dir {
                marker = if app.expanded_dirs.contains(&item.path) {
                    "-"
                } else {
                    "+"
                };
            }
            let indent = "  ".repeat(item.depth.min(8));
            let mut label = format!("{}{} {}", indent, marker, item.name);
            if label.len() > w as usize {
                label.truncate(w as usize);
            }

            if idx == app.explorer_index {
                queue!(
                    out,
                    SetForegroundColor(Color::Rgb { r: 10, g: 18, b: 30 }),
                    SetBackgroundColor(Color::Rgb { r: 132, g: 226, b: 255 }),
                    MoveTo(x, py),
                    Print(format!("{:<width$}", label, width = w as usize)),
                    SetBackgroundColor(bg),
                    SetForegroundColor(fg)
                )?;
            } else {
                queue!(out, Print(label))?;
            }
        }
    }

    Ok(())
}

fn draw_editor_pane(
    out: &mut Stdout,
    app: &mut App,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    fg: Color,
    accent: Color,
) -> io::Result<(u16, u16)> {
    if w == 0 || h < 2 {
        return Ok((x, y));
    }

    queue!(
        out,
        SetForegroundColor(accent),
        MoveTo(x, y.saturating_sub(1)),
        Print("Editor"),
        SetForegroundColor(fg)
    )?;

    let tab_line = app
        .buffers
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let dirty = if b.dirty { "*" } else { "" };
            if i == app.active_buffer {
                format!("[{}{}]", b.name, dirty)
            } else {
                format!(" {}{} ", b.name, dirty)
            }
        })
        .collect::<Vec<String>>()
        .join(" | ");
    let clipped_tabs: String = tab_line.chars().take(w as usize).collect();
    queue!(out, MoveTo(x, y), Print(clipped_tabs))?;

    let text_y = y + 1;
    let text_h = h.saturating_sub(1) as usize;

    let buffer = app.active_buffer_mut();

    if buffer.cursor_y < buffer.scroll_y {
        buffer.scroll_y = buffer.cursor_y;
    }
    if text_h > 0 && buffer.cursor_y >= buffer.scroll_y + text_h {
        buffer.scroll_y = buffer.cursor_y + 1 - text_h;
    }

    for row in 0..text_h {
        let line_idx = buffer.scroll_y + row;
        let py = text_y + row as u16;
        queue!(out, MoveTo(x, py))?;

        if let Some(line) = buffer.lines.get(line_idx) {
            let num = format!("{:>4} ", line_idx + 1);
            let avail = w.saturating_sub(num.len() as u16) as usize;
            let mut text = line.clone();
            if text.len() > avail {
                text.truncate(avail);
            }
            let merged = format!("{}{}", num, text);
            queue!(out, Print(merged))?;
        }
    }

    let cursor_row = buffer.cursor_y.saturating_sub(buffer.scroll_y);
    let cursor_y = text_y + cursor_row.min(text_h.saturating_sub(1)) as u16;
    let cursor_x = x + 5 + buffer.cursor_x.min(w.saturating_sub(6) as usize) as u16;

    Ok((cursor_x, cursor_y))
}

fn draw_status_line(
    out: &mut Stdout,
    app: &App,
    x: u16,
    y: u16,
    w: u16,
    status_bg: Color,
    status_fg: Color,
) -> io::Result<()> {
    let bindings = format!(
        "F2 folder  F1 file  F3 mkfile  F4 mkdir  F5 save  F6 save-as  F7 close  F9 pane  F10 quit",
    );
    let ctrl_bindings = format!(
        "^{} save  ^{} save-as  ^{} open  ^{} folder  ^{} mkfile  ^{} mkdir  ^{} close  ^{} pane  ^{} quit",
        K_SAVE,
        K_SAVE_AS,
        K_OPEN_FILE,
        K_OPEN_FOLDER,
        K_NEW_FILE,
        K_NEW_FOLDER,
        K_CLOSE_BUFFER,
        K_TOGGLE_FOCUS,
        K_QUIT
    );

    let right = if w > 110 { ctrl_bindings } else { bindings };
    let text = if w as usize > right.len() + 10 {
        let left_space = (w as usize).saturating_sub(right.len() + 3);
        let status_clipped: String = app.status.chars().take(left_space).collect();
        format!("{} | {}", status_clipped, right)
    } else {
        right
    };
    let clipped: String = text.chars().take(w as usize).collect();

    queue!(
        out,
        SetBackgroundColor(status_bg),
        SetForegroundColor(status_fg),
        MoveTo(x, y),
        Print(" ".repeat(w as usize)),
        MoveTo(x, y),
        Print(clipped)
    )?;

    Ok(())
}
