//! The window: Convert, Textures, Validate, Preview and Inspect tabs.
//!
//! Conversions and the other command line jobs run `msfs2xp.exe` (next to
//! this program) in the background; their output streams into the window
//! through a channel and a `Notice`, which wakes the UI thread. Texture
//! thumbnails are made on a worker thread the same way. Texture fixes are
//! applied directly with the converter's library.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use msfs2xp::fixes::{self, Fixes, TextureFix};
use native_windows_gui as nwg;

use crate::logic::{self, Settings};

/// Start child processes without a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const THUMB: u32 = 64;
const PREVIEW: u32 = 320;
const TEXTURE_SIZES: [u32; 4] = [4096, 2048, 1024, 512];
const NORMAL_SIZES: [u32; 3] = [256, 512, 1024];
const SIM_LAYOUTS: [&str; 5] = ["auto", "2020", "2024", "fsx", "p3d"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Job {
    Scan,
    Convert,
    Validate,
    Preview,
    Inspect,
}

enum Msg {
    Line(Job, String),
    Done(Job, Option<i32>),
    /// Thumbnail PNG for texture `index` of pack load `generation`.
    Thumb(u64, usize, Vec<u8>),
}

#[derive(Default)]
struct State {
    rx: Option<Receiver<Msg>>,
    tx: Option<Sender<Msg>>,
    child: Option<Arc<Mutex<Option<Child>>>>,
    running: Option<Job>,
    scan_text: String,
    scanned: Vec<(String, String)>,
    preview_html: Option<PathBuf>,
    pack: Option<PathBuf>,
    generation: u64,
    textures: Vec<String>,
    /// Texture index of each list row.
    shown: Vec<usize>,
    /// Image list slot of each texture's thumbnail, once made.
    thumb_slot: Vec<Option<i32>>,
    fixes: Fixes,
}

#[derive(Default)]
pub struct App {
    window: nwg::Window,
    notice: nwg::Notice,
    tabs: nwg::TabsContainer,
    tab_convert: nwg::Tab,
    tab_textures: nwg::Tab,
    tab_validate: nwg::Tab,
    tab_preview: nwg::Tab,
    tab_inspect: nwg::Tab,

    pkg_label: nwg::Label,
    packages: nwg::ListBox<String>,
    add_pkg: nwg::Button,
    add_2020: nwg::Button,
    add_2024: nwg::Button,
    remove_pkg: nwg::Button,
    scan: nwg::Button,
    airports_label: nwg::Label,
    airports: nwg::ListBox<String>,
    out_label: nwg::Label,
    out_path: nwg::TextInput,
    out_browse: nwg::Button,
    opt_objects: nwg::CheckBox,
    opt_normals: nwg::CheckBox,
    opt_merge: nwg::CheckBox,
    opt_lines: nwg::CheckBox,
    opt_network: nwg::CheckBox,
    opt_union: nwg::CheckBox,
    tex_label: nwg::Label,
    max_texture: nwg::ComboBox<String>,
    nm_label: nwg::Label,
    normal_max: nwg::ComboBox<String>,
    sim_label: nwg::Label,
    sim: nwg::ComboBox<String>,
    convert: nwg::Button,
    cancel: nwg::Button,
    open_out: nwg::Button,
    progress: nwg::ProgressBar,
    log: nwg::TextBox,

    open_pack: nwg::Button,
    pack_label: nwg::Label,
    filter_label: nwg::Label,
    filter: nwg::TextInput,
    tex_list: nwg::ListView,
    thumbs: RefCell<nwg::ImageList>,
    preview: nwg::ImageFrame,
    preview_bitmap: RefCell<Option<nwg::Bitmap>>,
    tex_name: nwg::Label,
    tex_info: nwg::Label,
    tex_fix: nwg::Label,
    flip_v: nwg::Button,
    flip_h: nwg::Button,
    rotate: nwg::Button,
    hide: nwg::Button,
    revert: nwg::Button,
    open_tex_dir: nwg::Button,
    tex_note: nwg::Label,

    val_path: nwg::TextInput,
    val_browse: nwg::Button,
    val_run: nwg::Button,
    val_out: nwg::TextBox,
    pre_path: nwg::TextInput,
    pre_browse: nwg::Button,
    pre_run: nwg::Button,
    pre_out: nwg::TextBox,
    ins_path: nwg::TextInput,
    ins_browse: nwg::Button,
    ins_icao_label: nwg::Label,
    ins_icao: nwg::TextInput,
    ins_hex: nwg::CheckBox,
    ins_run: nwg::Button,
    ins_save: nwg::Button,
    ins_out: nwg::TextBox,

    folder_dialog: nwg::FileDialog,
    apt_dialog: nwg::FileDialog,
    bgl_dialog: nwg::FileDialog,
    save_dialog: nwg::FileDialog,

    state: RefCell<State>,
    /// Thumbnail and preview sizes in screen pixels.
    thumb: u32,
    preview_px: u32,
}

/// Tab pages are painted white; text controls must match.
const PAGE: [u8; 3] = [255, 255, 255];

pub fn run() {
    // Draw at the screen's real resolution (crisp text at 125%/150%
    // scaling) rather than letting Windows stretch a 96 DPI bitmap. nwg
    // then scales control positions, but not fonts or bitmaps.
    #[allow(deprecated)]
    unsafe {
        nwg::set_dpi_awareness()
    };
    nwg::init().expect("could not start the Windows UI");
    let scale = nwg::scale_factor();
    let mut font = nwg::Font::default();
    if nwg::Font::builder().family("Segoe UI").size((16.0 * scale).round() as u32).build(&mut font).is_ok() {
        nwg::Font::set_global_default(Some(font));
    }
    let mut app = App { thumb: (THUMB as f64 * scale).round() as u32, preview_px: (PREVIEW as f64 * scale).round() as u32, ..App::default() };
    if let Err(e) = build(&mut app) {
        nwg::fatal_message("msfs2xp", &format!("Could not build the window: {e}"));
    }
    let (tx, rx) = channel();
    {
        let mut st = app.state.borrow_mut();
        st.tx = Some(tx);
        st.rx = Some(rx);
    }
    app.load_settings();
    let app = Rc::new(app);
    let events = app.clone();
    let handler = nwg::full_bind_event_handler(&app.window.handle, move |evt, _data, handle| {
        events.on_event(evt, &handle);
    });
    nwg::dispatch_thread_events();
    nwg::unbind_event_handler(&handler);
}

fn label(text: &str, pos: (i32, i32), size: (i32, i32), parent: &nwg::Tab, out: &mut nwg::Label) -> Result<(), nwg::NwgError> {
    nwg::Label::builder().text(text).position(pos).size(size).background_color(Some(PAGE)).parent(parent).build(out)
}

fn button(text: &str, pos: (i32, i32), size: (i32, i32), parent: &nwg::Tab, out: &mut nwg::Button) -> Result<(), nwg::NwgError> {
    nwg::Button::builder().text(text).position(pos).size(size).parent(parent).build(out)
}

fn check(text: &str, pos: (i32, i32), parent: &nwg::Tab, out: &mut nwg::CheckBox) -> Result<(), nwg::NwgError> {
    nwg::CheckBox::builder().text(text).position(pos).size((245, 24)).background_color(Some(PAGE)).parent(parent).build(out)
}

fn output_box(pos: (i32, i32), size: (i32, i32), parent: &nwg::Tab, out: &mut nwg::TextBox) -> Result<(), nwg::NwgError> {
    nwg::TextBox::builder()
        .position(pos)
        .size(size)
        .flags(nwg::TextBoxFlags::VISIBLE | nwg::TextBoxFlags::VSCROLL | nwg::TextBoxFlags::AUTOVSCROLL)
        .parent(parent)
        .build(out)?;
    out.set_readonly(true);
    Ok(())
}

fn combo(items: Vec<String>, pos: (i32, i32), parent: &nwg::Tab, out: &mut nwg::ComboBox<String>) -> Result<(), nwg::NwgError> {
    nwg::ComboBox::builder().collection(items).selected_index(Some(0)).position(pos).size((90, 26)).parent(parent).build(out)
}

fn build(a: &mut App) -> Result<(), nwg::NwgError> {
    nwg::Window::builder()
        .title("msfs2xp \u{2013} MSFS airports to X-Plane 12")
        .size((1000, 745))
        .position((180, 60))
        .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::MINIMIZE_BOX | nwg::WindowFlags::VISIBLE)
        .build(&mut a.window)?;
    nwg::Notice::builder().parent(&a.window).build(&mut a.notice)?;
    nwg::TabsContainer::builder().position((5, 5)).size((990, 735)).parent(&a.window).build(&mut a.tabs)?;
    for (tab, text) in [
        (&mut a.tab_convert, "Convert"),
        (&mut a.tab_textures, "Textures"),
        (&mut a.tab_validate, "Validate"),
        (&mut a.tab_preview, "Preview"),
        (&mut a.tab_inspect, "Inspect"),
    ] {
        nwg::Tab::builder().text(text).parent(&a.tabs).build(tab)?;
    }

    // Convert
    let t = &a.tab_convert;
    label("MSFS packages (package folders, or a whole MSFS install):", (10, 8), (590, 24), t, &mut a.pkg_label)?;
    nwg::ListBox::builder().position((10, 34)).size((590, 110)).parent(t).build(&mut a.packages)?;
    button("Add package folder\u{2026}", (610, 34), (190, 28), t, &mut a.add_pkg)?;
    button("Add MSFS 2020 install", (610, 66), (190, 28), t, &mut a.add_2020)?;
    button("Add MSFS 2024 install", (610, 98), (190, 28), t, &mut a.add_2024)?;
    button("Remove", (810, 34), (150, 28), t, &mut a.remove_pkg)?;
    button("Scan for airports", (810, 66), (150, 28), t, &mut a.scan)?;
    label("Airports (select some to convert only those; none selected converts all):", (10, 150), (590, 24), t, &mut a.airports_label)?;
    nwg::ListBox::builder()
        .position((10, 176))
        .size((590, 110))
        .flags(nwg::ListBoxFlags::VISIBLE | nwg::ListBoxFlags::MULTI_SELECT | nwg::ListBoxFlags::TAB_STOP)
        .parent(t)
        .build(&mut a.airports)?;
    label("Output folder:", (10, 296), (110, 24), t, &mut a.out_label)?;
    nwg::TextInput::builder().position((125, 292)).size((475, 32)).parent(t).build(&mut a.out_path)?;
    button("Browse\u{2026}", (610, 294), (190, 28), t, &mut a.out_browse)?;
    check("Buildings and objects", (10, 332), t, &mut a.opt_objects)?;
    check("Normal maps (surface detail)", (260, 332), t, &mut a.opt_normals)?;
    check("One merged pack", (510, 332), t, &mut a.opt_merge)?;
    check("Painted lines and lights", (10, 360), t, &mut a.opt_lines)?;
    check("ATC taxi network", (260, 360), t, &mut a.opt_network)?;
    check("Merge pavement", (510, 360), t, &mut a.opt_union)?;
    label("Max texture size:", (10, 395), (130, 24), t, &mut a.tex_label)?;
    combo(TEXTURE_SIZES.iter().map(u32::to_string).collect(), (145, 394), t, &mut a.max_texture)?;
    label("Normal map size:", (255, 395), (145, 24), t, &mut a.nm_label)?;
    combo(NORMAL_SIZES.iter().map(u32::to_string).collect(), (400, 394), t, &mut a.normal_max)?;
    label("Record layout:", (510, 395), (112, 24), t, &mut a.sim_label)?;
    combo(SIM_LAYOUTS.iter().map(|s| s.to_string()).collect(), (625, 394), t, &mut a.sim)?;
    button("Convert", (10, 436), (160, 34), t, &mut a.convert)?;
    button("Cancel", (180, 436), (110, 34), t, &mut a.cancel)?;
    button("Open output folder", (300, 436), (170, 34), t, &mut a.open_out)?;
    nwg::ProgressBar::builder().position((485, 442)).size((475, 22)).range(0..100).parent(t).build(&mut a.progress)?;
    output_box((10, 480), (950, 215), t, &mut a.log)?;
    a.cancel.set_enabled(false);

    // Textures
    let t = &a.tab_textures;
    button("Open converted pack\u{2026}", (10, 10), (190, 28), t, &mut a.open_pack)?;
    label("No pack open. Pick a converted airport folder (the one holding \u{201c}objects\u{201d}).", (210, 13), (750, 24), t, &mut a.pack_label)?;
    label("Search:", (10, 48), (60, 24), t, &mut a.filter_label)?;
    nwg::TextInput::builder().position((75, 45)).size((300, 32)).parent(t).build(&mut a.filter)?;
    nwg::ListView::builder()
        .position((10, 80))
        .size((560, 615))
        .list_style(nwg::ListViewStyle::Icon)
        .flags(
            nwg::ListViewFlags::VISIBLE
                | nwg::ListViewFlags::SINGLE_SELECTION
                | nwg::ListViewFlags::ALWAYS_SHOW_SELECTION
                | nwg::ListViewFlags::TAB_STOP,
        )
        .parent(t)
        .build(&mut a.tex_list)?;
    nwg::ImageList::builder().size((a.thumb as i32, a.thumb as i32)).initial(64).grow(64).build(&mut a.thumbs.borrow_mut())?;
    a.tex_list.set_image_list(Some(&a.thumbs.borrow()), nwg::ListViewImageListType::Normal);
    nwg::ImageFrame::builder()
        .position((585, 47))
        .size((PREVIEW as i32, PREVIEW as i32))
        .background_color(Some([38, 38, 42]))
        .parent(t)
        .build(&mut a.preview)?;
    label("", (585, 374), (380, 24), t, &mut a.tex_name)?;
    label("", (585, 398), (380, 24), t, &mut a.tex_info)?;
    label("", (585, 422), (380, 24), t, &mut a.tex_fix)?;
    button("Flip top to bottom", (585, 450), (185, 32), t, &mut a.flip_v)?;
    button("Flip left to right", (780, 450), (185, 32), t, &mut a.flip_h)?;
    button("Turn 180\u{b0}", (585, 487), (185, 32), t, &mut a.rotate)?;
    button("Hide / unhide", (780, 487), (185, 32), t, &mut a.hide)?;
    button("Revert to original", (585, 524), (185, 32), t, &mut a.revert)?;
    button("Open textures folder", (780, 524), (185, 32), t, &mut a.open_tex_dir)?;
    label(
        "Fixes change the pack right away and are saved in its msfs2xp-fixes.json, so converting the airport again keeps them. Originals are kept in objects\\textures\\_originals.",
        (585, 566),
        (380, 125),
        t,
        &mut a.tex_note,
    )?;

    // Validate, Preview, Inspect
    let t = &a.tab_validate;
    nwg::TextInput::builder().position((10, 10)).size((740, 32)).parent(t).build(&mut a.val_path)?;
    button("Browse\u{2026}", (760, 10), (100, 32), t,&mut a.val_browse)?;
    button("Validate", (870, 10), (100, 32), t, &mut a.val_run)?;
    output_box((10, 50), (960, 645), t, &mut a.val_out)?;
    let t = &a.tab_preview;
    nwg::TextInput::builder().position((10, 10)).size((740, 32)).parent(t).build(&mut a.pre_path)?;
    button("Browse\u{2026}", (760, 10), (100, 32), t,&mut a.pre_browse)?;
    button("Render map", (870, 10), (100, 32), t, &mut a.pre_run)?;
    output_box((10, 50), (960, 645), t, &mut a.pre_out)?;
    let t = &a.tab_inspect;
    nwg::TextInput::builder().position((10, 10)).size((740, 32)).parent(t).build(&mut a.ins_path)?;
    button("Browse\u{2026}", (760, 10), (100, 32), t,&mut a.ins_browse)?;
    button("Inspect", (870, 10), (100, 32), t, &mut a.ins_run)?;
    label("Only airport (ICAO):", (10, 49), (150, 24), t, &mut a.ins_icao_label)?;
    nwg::TextInput::builder().position((165, 46)).size((90, 32)).parent(t).build(&mut a.ins_icao)?;
    nwg::CheckBox::builder()
        .text("Hex dump unknown records")
        .position((275, 48))
        .size((260, 26))
        .background_color(Some(PAGE))
        .parent(t)
        .build(&mut a.ins_hex)?;
    button("Save output as\u{2026}", (820, 47), (150, 28), t, &mut a.ins_save)?;
    output_box((10, 84), (960, 611), t, &mut a.ins_out)?;

    nwg::FileDialog::builder()
        .title("Choose a folder")
        .action(nwg::FileDialogAction::OpenDirectory)
        .build(&mut a.folder_dialog)?;
    nwg::FileDialog::builder()
        .title("Choose an apt.dat")
        .action(nwg::FileDialogAction::Open)
        .filters("Airport data(*.dat)|All files(*.*)")
        .build(&mut a.apt_dialog)?;
    nwg::FileDialog::builder()
        .title("Choose a BGL file")
        .action(nwg::FileDialogAction::Open)
        .filters("BGL files(*.bgl)|All files(*.*)")
        .build(&mut a.bgl_dialog)?;
    nwg::FileDialog::builder()
        .title("Save output as")
        .action(nwg::FileDialogAction::Save)
        .filters("Text(*.txt)|All files(*.*)")
        .build(&mut a.save_dialog)?;
    Ok(())
}

fn checked(c: &nwg::CheckBox) -> bool {
    c.check_state() == nwg::CheckBoxState::Checked
}

fn set_checked(c: &nwg::CheckBox, on: bool) {
    c.set_check_state(if on { nwg::CheckBoxState::Checked } else { nwg::CheckBoxState::Unchecked });
}

fn open_in_explorer(path: &Path) {
    let _ = Command::new("explorer").arg(path).spawn();
}

impl App {
    fn load_settings(&self) {
        let s = logic::load_settings();
        self.packages.set_collection(s.packages.clone());
        self.out_path.set_text(&s.output);
        set_checked(&self.opt_objects, s.objects);
        set_checked(&self.opt_normals, s.normal_maps);
        set_checked(&self.opt_merge, s.merge);
        set_checked(&self.opt_lines, s.lines);
        set_checked(&self.opt_network, s.network);
        set_checked(&self.opt_union, s.union);
        self.max_texture.set_selection_string(&s.max_texture.to_string());
        self.normal_max.set_selection_string(&s.normal_max.to_string());
        self.sim.set_selection_string(&s.sim);
    }

    fn settings(&self) -> Settings {
        let pick = |c: &nwg::ComboBox<String>, default: u32| {
            c.selection_string().and_then(|v| v.parse().ok()).unwrap_or(default)
        };
        Settings {
            packages: self.packages.collection().clone(),
            output: self.out_path.text().trim().to_string(),
            objects: checked(&self.opt_objects),
            normal_maps: checked(&self.opt_normals),
            merge: checked(&self.opt_merge),
            lines: checked(&self.opt_lines),
            network: checked(&self.opt_network),
            union: checked(&self.opt_union),
            max_texture: pick(&self.max_texture, 2048),
            normal_max: pick(&self.normal_max, 512),
            sim: self.sim.selection_string().unwrap_or_else(|| "auto".into()),
        }
    }

    fn error(&self, text: &str) {
        nwg::modal_error_message(&self.window, "msfs2xp", text);
    }

    fn pick_folder(&self) -> Option<String> {
        if self.folder_dialog.run(Some(&self.window)) {
            self.folder_dialog.get_selected_item().ok().map(|p| p.to_string_lossy().to_string())
        } else {
            None
        }
    }

    fn pick_file(&self, dialog: &nwg::FileDialog) -> Option<String> {
        if dialog.run(Some(&self.window)) {
            dialog.get_selected_item().ok().map(|p| p.to_string_lossy().to_string())
        } else {
            None
        }
    }

    fn on_event(&self, evt: nwg::Event, handle: &nwg::ControlHandle) {
        use nwg::Event as E;
        match evt {
            E::OnWindowClose if handle == &self.window.handle => {
                logic::save_settings(&self.settings());
                self.kill_job();
                nwg::stop_thread_dispatch();
            }
            E::OnNotice if handle == &self.notice.handle => self.drain_messages(),
            E::OnButtonClick => self.on_click(handle),
            E::OnListViewItemChanged | E::OnListViewClick if handle == &self.tex_list.handle => self.show_selected(),
            E::OnTextInput if handle == &self.filter.handle => self.refill_list(),
            _ => {}
        }
    }

    fn on_click(&self, h: &nwg::ControlHandle) {
        if h == &self.add_pkg.handle {
            if let Some(dir) = self.pick_folder() {
                if !self.packages.collection().contains(&dir) {
                    self.packages.push(dir);
                }
            }
        } else if h == &self.add_2020.handle {
            self.packages.push("auto:2020".into());
        } else if h == &self.add_2024.handle {
            self.packages.push("auto:2024".into());
        } else if h == &self.remove_pkg.handle {
            if let Some(i) = self.packages.selection() {
                self.packages.remove(i);
            }
        } else if h == &self.scan.handle {
            let inputs = self.packages.collection().clone();
            if inputs.is_empty() {
                return self.error("Add a package folder or an MSFS install first.");
            }
            self.airports.clear();
            {
                let mut st = self.state.borrow_mut();
                st.scan_text.clear();
                st.scanned.clear();
            }
            let mut args = vec!["list".to_string()];
            args.extend(inputs);
            self.log.appendln("Scanning packages for airports\u{2026}");
            self.start_job(Job::Scan, args);
        } else if h == &self.out_browse.handle {
            if let Some(dir) = self.pick_folder() {
                self.out_path.set_text(&dir);
            }
        } else if h == &self.convert.handle {
            self.start_convert();
        } else if h == &self.cancel.handle {
            self.kill_job();
        } else if h == &self.open_out.handle {
            open_in_explorer(Path::new(self.out_path.text().trim()));
        } else if h == &self.open_pack.handle {
            if let Some(dir) = self.pick_folder() {
                self.open_texture_pack(PathBuf::from(dir));
            }
        } else if h == &self.flip_v.handle {
            self.change_fix(|f| f.flip_v = !f.flip_v);
        } else if h == &self.flip_h.handle {
            self.change_fix(|f| f.flip_h = !f.flip_h);
        } else if h == &self.rotate.handle {
            self.change_fix(|f| {
                f.flip_v = !f.flip_v;
                f.flip_h = !f.flip_h;
            });
        } else if h == &self.hide.handle {
            self.change_fix(|f| f.hide = !f.hide);
        } else if h == &self.revert.handle {
            self.change_fix(|f| *f = TextureFix::default());
        } else if h == &self.open_tex_dir.handle {
            if let Some(pack) = self.state.borrow().pack.clone() {
                open_in_explorer(&pack.join("objects").join("textures"));
            }
        } else if h == &self.val_browse.handle {
            if let Some(f) = self.pick_file(&self.apt_dialog) {
                self.val_path.set_text(&f);
            }
        } else if h == &self.val_run.handle {
            let p = self.val_path.text();
            if p.trim().is_empty() {
                return self.error("Pick an apt.dat to check.");
            }
            self.val_out.clear();
            self.start_job(Job::Validate, vec!["validate".into(), p.trim().to_string()]);
        } else if h == &self.pre_browse.handle {
            if let Some(f) = self.pick_file(&self.apt_dialog) {
                self.pre_path.set_text(&f);
            }
        } else if h == &self.pre_run.handle {
            let apt = PathBuf::from(self.pre_path.text().trim());
            if !apt.is_file() {
                return self.error("Pick an apt.dat to draw.");
            }
            let html = apt.with_file_name("apt-preview.html");
            self.state.borrow_mut().preview_html = Some(html.clone());
            self.pre_out.clear();
            self.start_job(
                Job::Preview,
                vec!["preview".into(), apt.display().to_string(), "-o".into(), html.display().to_string()],
            );
        } else if h == &self.ins_browse.handle {
            if let Some(f) = self.pick_file(&self.bgl_dialog) {
                self.ins_path.set_text(&f);
            }
        } else if h == &self.ins_run.handle {
            let p = self.ins_path.text();
            if p.trim().is_empty() {
                return self.error("Pick a BGL file to inspect.");
            }
            let mut args = vec!["inspect".to_string(), p.trim().to_string()];
            let icao = self.ins_icao.text();
            if !icao.trim().is_empty() {
                args.extend(["--icao".to_string(), icao.trim().to_uppercase()]);
            }
            if checked(&self.ins_hex) {
                args.push("--hex".into());
            }
            self.ins_out.clear();
            self.start_job(Job::Inspect, args);
        } else if h == &self.ins_save.handle {
            if let Some(f) = self.pick_file(&self.save_dialog) {
                let path = if Path::new(&f).extension().is_none() { format!("{f}.txt") } else { f };
                if let Err(e) = std::fs::write(&path, self.ins_out.text()) {
                    self.error(&format!("Could not save {path}: {e}"));
                }
            }
        }
    }

    fn start_convert(&self) {
        let s = self.settings();
        if s.packages.is_empty() {
            return self.error("Add a package folder or an MSFS install first.");
        }
        if s.output.is_empty() {
            return self.error("Choose an output folder.");
        }
        logic::save_settings(&s);
        let icaos: Vec<String> = {
            let st = self.state.borrow();
            self.airports
                .multi_selection()
                .into_iter()
                .filter_map(|i| st.scanned.get(i).map(|a| a.0.clone()))
                .collect()
        };
        self.log.clear();
        let args = logic::convert_args(&s, &icaos);
        self.log.appendln(&format!("msfs2xp {}", args.join(" ")));
        self.start_job(Job::Convert, args);
    }

    /// Run the command line tool with `args`, streaming its output.
    fn start_job(&self, job: Job, args: Vec<String>) {
        if let Some(running) = self.state.borrow().running {
            return self.error(&format!("Wait for the current job ({running:?}) to finish, or cancel it."));
        }
        let cli = logic::cli_path();
        if !cli.is_file() {
            return self.error(&format!("msfs2xp.exe was not found next to this program ({}).", cli.display()));
        }
        let mut child = match Command::new(&cli)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return self.error(&format!("Could not start msfs2xp.exe: {e}")),
        };
        let Some(tx) = self.state.borrow().tx.clone() else { return };
        let sender = self.notice.sender();
        let pipes: Vec<Box<dyn Read + Send>> = [
            child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
            child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .collect();
        for pipe in pipes {
            let (tx, sender) = (tx.clone(), sender);
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(Msg::Line(job, line)).is_err() {
                        break;
                    }
                    sender.notice();
                }
            });
        }
        let child = Arc::new(Mutex::new(Some(child)));
        let waiter = child.clone();
        std::thread::spawn(move || loop {
            let status = {
                let mut guard = match waiter.lock() {
                    Ok(g) => g,
                    Err(_) => break,
                };
                match guard.as_mut().map(Child::try_wait) {
                    Some(Ok(Some(status))) => Some(status.code()),
                    Some(Ok(None)) => None,
                    _ => Some(None),
                }
            };
            if let Some(code) = status {
                // Let the output threads finish their last lines first.
                std::thread::sleep(Duration::from_millis(150));
                let _ = tx.send(Msg::Done(job, code));
                sender.notice();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        });
        {
            let mut st = self.state.borrow_mut();
            st.child = Some(child);
            st.running = Some(job);
        }
        if job == Job::Convert {
            self.progress.add_flags(nwg::ProgressBarFlags::MARQUEE);
            self.progress.set_marquee(true, 30);
        }
        self.convert.set_enabled(false);
        self.scan.set_enabled(false);
        self.cancel.set_enabled(true);
    }

    fn kill_job(&self) {
        let child = self.state.borrow().child.clone();
        if let Some(child) = child {
            if let Ok(mut guard) = child.lock() {
                if let Some(c) = guard.as_mut() {
                    let _ = c.kill();
                }
            }
        }
    }

    fn output_for(&self, job: Job) -> &nwg::TextBox {
        match job {
            Job::Scan | Job::Convert => &self.log,
            Job::Validate => &self.val_out,
            Job::Preview => &self.pre_out,
            Job::Inspect => &self.ins_out,
        }
    }

    fn drain_messages(&self) {
        loop {
            let msg = {
                let st = self.state.borrow();
                match st.rx.as_ref().map(Receiver::try_recv) {
                    Some(Ok(m)) => m,
                    _ => break,
                }
            };
            match msg {
                Msg::Line(job, line) => {
                    if job == Job::Scan {
                        let mut st = self.state.borrow_mut();
                        st.scan_text.push_str(&line);
                        st.scan_text.push('\n');
                    }
                    self.output_for(job).appendln(&line);
                }
                Msg::Done(job, code) => self.job_done(job, code),
                Msg::Thumb(generation, index, png) => self.add_thumbnail(generation, index, &png),
            }
        }
    }

    fn job_done(&self, job: Job, code: Option<i32>) {
        {
            let mut st = self.state.borrow_mut();
            st.running = None;
            st.child = None;
        }
        self.progress.set_marquee(false, 0);
        self.progress.remove_flags(nwg::ProgressBarFlags::MARQUEE);
        self.progress.set_pos(if code == Some(0) { 100 } else { 0 });
        self.convert.set_enabled(true);
        self.scan.set_enabled(true);
        self.cancel.set_enabled(false);
        let result = match code {
            Some(0) => "finished".to_string(),
            Some(c) => format!("finished with exit code {c}"),
            None => "stopped".to_string(),
        };
        match job {
            Job::Scan => {
                let found = logic::parse_list(&self.state.borrow().scan_text);
                let rows: Vec<String> = found.iter().map(|(icao, name)| format!("{icao}   {name}")).collect();
                self.airports.set_collection(rows);
                self.log.appendln(&format!("Scan {result}: {} airport(s) found.", found.len()));
                self.state.borrow_mut().scanned = found;
            }
            Job::Convert => self.log.appendln(&format!("Conversion {result}.")),
            Job::Preview => {
                self.pre_out.appendln(&format!("Preview {result}."));
                if code == Some(0) {
                    if let Some(html) = self.state.borrow().preview_html.clone() {
                        let _ = Command::new("cmd")
                            .args(["/C", "start", ""])
                            .arg(&html)
                            .creation_flags(CREATE_NO_WINDOW)
                            .spawn();
                    }
                }
            }
            Job::Validate | Job::Inspect => self.output_for(job).appendln(&format!("\u{2014} {result}")),
        }
    }

    fn open_texture_pack(&self, pack: PathBuf) {
        let textures = match logic::list_textures(&pack) {
            Ok(t) => t,
            Err(_) => {
                return self.error("That folder has no objects\\textures. Pick the converted airport folder, e.g. \u{201c}OMDB Dubai International Airport (msfs2xp)\u{201d}.")
            }
        };
        let fixes = Fixes::load(&pack.join(fixes::FILE_NAME)).unwrap_or_default();
        // A fresh image list for the new pack's thumbnails.
        let mut list = nwg::ImageList::default();
        if nwg::ImageList::builder()
            .size((self.thumb as i32, self.thumb as i32))
            .initial(textures.len().max(1) as i32)
            .grow(64)
            .build(&mut list)
            .is_ok()
        {
            self.tex_list.set_image_list(Some(&list), nwg::ListViewImageListType::Normal);
            *self.thumbs.borrow_mut() = list;
        }
        let generation = {
            let mut st = self.state.borrow_mut();
            st.generation += 1;
            st.pack = Some(pack.clone());
            st.thumb_slot = vec![None; textures.len()];
            st.textures = textures.clone();
            st.fixes = fixes;
            st.generation
        };
        self.pack_label.set_text(&format!("{}  \u{2014}  {} textures", pack.display(), textures.len()));
        self.refill_list();
        self.clear_details();

        // Thumbnails on a worker thread.
        let Some(tx) = self.state.borrow().tx.clone() else { return };
        let sender = self.notice.sender();
        let dir = pack.join("objects").join("textures");
        let side = self.thumb;
        std::thread::spawn(move || {
            for (i, name) in textures.iter().enumerate() {
                if let Ok(png) = logic::thumbnail_png(&dir.join(name), side) {
                    if tx.send(Msg::Thumb(generation, i, png)).is_err() {
                        return;
                    }
                }
                if i % 8 == 7 {
                    sender.notice();
                }
            }
            sender.notice();
        });
    }

    fn add_thumbnail(&self, generation: u64, index: usize, png: &[u8]) {
        if generation != self.state.borrow().generation {
            return;
        }
        let Ok(bitmap) = nwg::Bitmap::from_bin(png) else { return };
        let slot = self.thumbs.borrow().add_bitmap(&bitmap);
        let row = {
            let mut st = self.state.borrow_mut();
            if let Some(s) = st.thumb_slot.get_mut(index) {
                *s = Some(slot);
            }
            st.shown.iter().position(|&i| i == index)
        };
        if let Some(row) = row {
            self.update_row(row, index);
        }
    }

    fn row_text(&self, index: usize) -> String {
        let st = self.state.borrow();
        let name = st.textures.get(index).cloned().unwrap_or_default();
        let fix = st.fixes.get(&name);
        if fix.is_none() {
            name
        } else {
            format!("{name} *")
        }
    }

    fn update_row(&self, row: usize, index: usize) {
        let image = self.state.borrow().thumb_slot.get(index).copied().flatten();
        self.tex_list.update_item(
            row,
            nwg::InsertListViewItem {
                index: Some(row as i32),
                column_index: 0,
                text: Some(self.row_text(index)),
                image,
            },
        );
    }

    /// Rebuild the texture list from the search box.
    fn refill_list(&self) {
        let needle = self.filter.text().to_ascii_lowercase();
        let shown: Vec<usize> = {
            let st = self.state.borrow();
            st.textures
                .iter()
                .enumerate()
                .filter(|(_, n)| needle.is_empty() || n.to_ascii_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect()
        };
        self.tex_list.set_redraw(false);
        self.tex_list.clear();
        for (row, &index) in shown.iter().enumerate() {
            let image = self.state.borrow().thumb_slot.get(index).copied().flatten();
            self.tex_list.insert_item(nwg::InsertListViewItem {
                index: Some(row as i32),
                column_index: 0,
                text: Some(self.row_text(index)),
                image,
            });
        }
        self.tex_list.set_redraw(true);
        self.state.borrow_mut().shown = shown;
    }

    fn clear_details(&self) {
        self.preview.set_bitmap(None);
        *self.preview_bitmap.borrow_mut() = None;
        self.tex_name.set_text("");
        self.tex_info.set_text("");
        self.tex_fix.set_text("Pick a texture to see it and fix it.");
    }

    /// The selected texture: (index, file name).
    fn selected(&self) -> Option<(usize, String)> {
        let row = self.tex_list.selected_item()?;
        let st = self.state.borrow();
        let index = *st.shown.get(row)?;
        Some((index, st.textures.get(index)?.clone()))
    }

    fn show_selected(&self) {
        let Some((_, name)) = self.selected() else { return };
        let Some(pack) = self.state.borrow().pack.clone() else { return };
        let path = pack.join("objects").join("textures").join(&name);
        match logic::thumbnail_png(&path, self.preview_px).ok().and_then(|png| nwg::Bitmap::from_bin(&png).ok()) {
            Some(bitmap) => {
                self.preview.set_bitmap(Some(&bitmap));
                *self.preview_bitmap.borrow_mut() = Some(bitmap);
            }
            None => self.preview.set_bitmap(None),
        }
        self.tex_name.set_text(&name);
        self.tex_info.set_text(&logic::texture_info(&path));
        let fix = self.state.borrow().fixes.get(&name);
        self.tex_fix.set_text(&logic::describe(&fix));
    }

    /// Change the selected texture's fix and apply it to the pack.
    fn change_fix(&self, edit: impl FnOnce(&mut TextureFix)) {
        let Some((index, name)) = self.selected() else {
            return self.error("Pick a texture first.");
        };
        let Some(pack) = self.state.borrow().pack.clone() else { return };
        let mut fix = self.state.borrow().fixes.get(&name);
        edit(&mut fix);
        match fixes::apply_to_pack(&pack, &name, fix) {
            Ok(objects) => {
                self.state.borrow_mut().fixes.set(&name, fix);
                let dir = pack.join("objects").join("textures");
                if let Ok(png) = logic::thumbnail_png(&dir.join(&name), self.thumb) {
                    if let Ok(bitmap) = nwg::Bitmap::from_bin(&png) {
                        let slot = self.state.borrow().thumb_slot.get(index).copied().flatten();
                        match slot {
                            Some(s) => self.thumbs.borrow().replace_bitmap(s, &bitmap),
                            None => {
                                let s = self.thumbs.borrow().add_bitmap(&bitmap);
                                if let Some(t) = self.state.borrow_mut().thumb_slot.get_mut(index) {
                                    *t = Some(s);
                                }
                            }
                        }
                    }
                }
                if let Some(row) = self.tex_list.selected_item() {
                    self.update_row(row, index);
                }
                self.tex_list.invalidate();
                self.show_selected();
                if objects > 0 {
                    let what = if fix.hide { "hidden" } else { "restored" };
                    self.tex_fix.set_text(&format!("{} \u{2014} {objects} object(s) {what}", logic::describe(&fix)));
                }
            }
            Err(e) => self.error(&format!("Could not change {name}: {e}")),
        }
    }
}
