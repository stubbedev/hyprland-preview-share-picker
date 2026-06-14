use std::{cell::RefCell, process::exit, rc::Rc};

use glib::variant::StaticVariantType;
use gtk4::{
    Application, ApplicationWindow, Box, CheckButton, CssProvider, EventControllerKey, FlowBox, Notebook,
    STYLE_PROVIDER_PRIORITY_APPLICATION, SearchEntry, Widget,
    gdk::Display,
    gio::{
        ActionEntry,
        prelude::{ActionMapExtManual, ApplicationExt, ApplicationExtManual},
    },
    glib::{ExitCode, clone, object::IsA},
    prelude::{BoxExt, Cast, CheckButtonExt, EditableExt, GtkWindowExt, WidgetExt},
};
use gtk4_layer_shell::*;
use hyprland_preview_share_picker_lib::toplevel::Toplevel;
use rsass::{compile_scss, output};
use wayland_client::Connection;

use crate::{
    config::{self, Config},
    views::{View, outputs::OutputsView, region::RegionView, windows::WindowsView},
};

const APP_ID: &str = "ch.wysbd.hyprland-preview-share-picker";

pub struct App {
    gtk_app: Application,
}

impl App {
    pub fn build(interactive_debug: bool, config: Config, toplevels: Vec<Toplevel>, restore_token: bool) -> Self {
        let gtk_app = Application::builder().application_id(APP_ID).build();

        let app = Self { gtk_app };

        app.gtk_app.connect_startup(clone!(
            #[strong]
            config,
            move |_| {
                load_stylesheets(&config);
            }
        ));

        if interactive_debug {
            // SAFETY: single-threaded startup, before any other thread is spawned.
            if let Err(err) = unsafe { gtk4::glib::setenv("GTK_DEBUG", "interactive", true) } {
                log::error!("unable to open gtk interactive debugger: {err}")
            } else {
                log::info!("opened interactive debugger")
            }
        }

        app.gtk_app.connect_activate(move |app| {
            log::debug!("gtk app is activated");
            build_ui(app, &config, &toplevels, restore_token);
        });

        app
    }

    pub fn run(&self) -> ExitCode {
        let empty_args: Vec<String> = vec![];
        self.gtk_app.run_with_args(&empty_args)
    }
}

fn build_ui(app: &Application, config: &Config, toplevels: &[Toplevel], default_restore_token: bool) {
    let window = build_window(app, config);
    log::debug!("built application window");
    let window_container = Box::new(gtk4::Orientation::Vertical, 0);
    window.set_child(Some(&window_container));

    let con = match Connection::connect_to_env() {
        Ok(connection) => connection,
        Err(err) => {
            log::error!("unable to connect to wayland server: {err}");
            exit(1);
        }
    };

    let restore_token = Rc::new(RefCell::new(default_restore_token));
    let exit_action = ActionEntry::builder("select")
        .parameter_type(Some(&String::static_variant_type()))
        .activate(clone!(
            #[strong]
            restore_token,
            move |_: &ApplicationWindow, _, parameter| {
                let allow_restore_token = *restore_token.borrow();
                let parameter = parameter
                    .expect("win.select called without parameter")
                    .get::<String>()
                    .expect("parameter of win.select action should be a string");
                println!("[SELECTION]{}/{parameter}", if allow_restore_token { "r" } else { "" });
                exit(0);
            }
        ))
        .build();
    window.add_action_entries([exit_action]);

    let notebook = Notebook::builder().css_classes([config.classes.notebook.as_str()]).vexpand(true).build();

    // Captured so the search entry below can filter the windows grid.
    let windows_flowbox: Rc<RefCell<Option<FlowBox>>> = Rc::new(RefCell::new(None));
    let mut windows_page_num: Option<u32> = None;

    match WindowsView::new(&con, toplevels, config) {
        Ok(view) => {
            let page = view.build();
            *windows_flowbox.borrow_mut() = page.child().and_then(|child| child.downcast::<FlowBox>().ok());
            let page_num = notebook.append_page(&page, Some(&view.label()));
            windows_page_num = Some(page_num);
            if let config::Page::Windows = config.default_page {
                notebook.set_current_page(Some(page_num));
            }
        }
        Err(err) => log::error!("unable to build windows view: {err}"),
    };

    match OutputsView::new(&con, config) {
        Ok(view) => {
            let page_num = notebook.append_page(&view.build(), Some(&view.label()));
            if let config::Page::Outputs = config.default_page {
                notebook.set_current_page(Some(page_num));
            }
        }
        Err(err) => log::error!("unable to build outputs view: {err}"),
    }

    match RegionView::new(config) {
        Ok(view) => {
            let page_num = notebook.append_page(&view.build(), Some(&view.label()));
            if let config::Page::Region = config.default_page {
                notebook.set_current_page(Some(page_num));
            }
        }
        Err(err) => log::error!("unable to build region view: {err}"),
    };

    // Live type-to-filter for the windows grid, with keyboard selection.
    let query = Rc::new(RefCell::new(String::new()));
    let search_entry = SearchEntry::builder()
        .placeholder_text("Search windows…")
        .css_classes([config.classes.search_entry.as_str()])
        .build();

    if let Some(flowbox) = windows_flowbox.borrow().as_ref() {
        flowbox.set_filter_func(clone!(
            #[strong]
            query,
            move |child| {
                let needle = query.borrow();
                needle.is_empty() || child.widget_name().as_str().contains(needle.as_str())
            }
        ));
    }

    search_entry.connect_search_changed(clone!(
        #[strong]
        windows_flowbox,
        #[strong]
        query,
        move |entry| {
            *query.borrow_mut() = entry.text().to_lowercase();
            if let Some(flowbox) = windows_flowbox.borrow().as_ref() {
                flowbox.invalidate_filter();
            }
        }
    ));

    // Enter activates the first visible match.
    search_entry.connect_activate(clone!(
        #[strong]
        windows_flowbox,
        #[strong]
        query,
        move |_| {
            if let Some(flowbox) = windows_flowbox.borrow().as_ref() {
                let needle = query.borrow();
                let mut index = 0;
                while let Some(child) = flowbox.child_at_index(index) {
                    if needle.is_empty() || child.widget_name().as_str().contains(needle.as_str()) {
                        child.activate();
                        break;
                    }
                    index += 1;
                }
            }
        }
    ));

    // Down moves focus from the search entry into the grid for arrow-key nav.
    let down_controller = EventControllerKey::new();
    down_controller.connect_key_pressed(clone!(
        #[strong]
        windows_flowbox,
        #[strong]
        query,
        move |_, key, _, _| {
            if let gtk4::gdk::Key::Down = key
                && let Some(flowbox) = windows_flowbox.borrow().as_ref()
            {
                let needle = query.borrow();
                let mut index = 0;
                while let Some(child) = flowbox.child_at_index(index) {
                    if needle.is_empty() || child.widget_name().as_str().contains(needle.as_str()) {
                        child.grab_focus();
                        return gtk4::glib::Propagation::Stop;
                    }
                    index += 1;
                }
            }
            gtk4::glib::Propagation::Proceed
        }
    ));
    search_entry.add_controller(down_controller);

    // Alt+1/2/3 jump between tabs without clashing with typing in the search entry.
    let tab_controller = EventControllerKey::new();
    tab_controller.connect_key_pressed(clone!(
        #[strong]
        notebook,
        move |_, key, _, state| {
            if state.contains(gtk4::gdk::ModifierType::ALT_MASK) {
                let page = match key {
                    gtk4::gdk::Key::_1 | gtk4::gdk::Key::KP_1 => Some(0),
                    gtk4::gdk::Key::_2 | gtk4::gdk::Key::KP_2 => Some(1),
                    gtk4::gdk::Key::_3 | gtk4::gdk::Key::KP_3 => Some(2),
                    _ => None,
                };
                if let Some(page) = page {
                    notebook.set_current_page(Some(page));
                    return gtk4::glib::Propagation::Stop;
                }
            }
            gtk4::glib::Propagation::Proceed
        }
    ));
    window.add_controller(tab_controller);

    // Search only applies to the windows grid — hide it on the other tabs.
    let on_windows_page = matches!(config.default_page, config::Page::Windows) && windows_page_num.is_some();
    search_entry.set_visible(on_windows_page);
    notebook.connect_switch_page(clone!(
        #[strong]
        search_entry,
        move |_, _, page_num| {
            let is_windows = windows_page_num == Some(page_num);
            search_entry.set_visible(is_windows);
            if is_windows {
                search_entry.grab_focus();
            }
        }
    ));

    window_container.append(&search_entry);
    window_container.append(&notebook);

    if !config.hide_token_restore {
        log::debug!("building token restore widget");
        let restore_button = build_restore_checkbox(restore_token, config);
        window_container.append(&restore_button);
    }

    log::debug!("presenting window");
    window.present();

    // Focus the search entry so the user can type-to-filter immediately.
    if on_windows_page {
        search_entry.grab_focus();
    }
}

fn load_stylesheets(config: &Config) {
    let provider = CssProvider::new();
    let format = output::Format { style: output::Style::Expanded, ..Default::default() };

    config.stylesheets.iter().for_each(|path_str| {
        let path = &config.resolve_path(path_str);
        if path.exists() {
            match std::fs::read(path) {
                Ok(content) => {
                    let css = if path.extension().is_some_and(|ext| ext == "scss") {
                        match compile_scss(content.as_slice(), format) {
                            Ok(css) => css,
                            Err(err) => {
                                log::error!("unable to compile stylesheet {path_str}: {err}");
                                Vec::new()
                            }
                        }
                    } else {
                        content
                    };
                    let str = std::str::from_utf8(css.as_slice()).expect("should be valid utf-8");
                    provider.load_from_data(str);
                }
                Err(err) => log::error!("unable to read stylesheet from {path_str}: {err}"),
            }
        } else {
            log::warn!("style path {path_str} does not exist");
        }
    });

    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("should have display"),
        &provider,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    )
}

fn build_window(app: &Application, config: &Config) -> ApplicationWindow {
    let window = ApplicationWindow::builder()
        .application(app)
        .destroy_with_parent(true)
        .default_width(config.window.width)
        .default_height(config.window.height)
        .vexpand(false)
        .hexpand(false)
        .css_classes([config.classes.window.as_str()])
        .build();

    let event_controller = EventControllerKey::new();
    event_controller.connect_key_pressed(|_, key, _, _| {
        if let gtk4::gdk::Key::Escape = key {
            log::debug!("exiting: escape key pressed");
            exit(0);
        }
        gtk4::glib::Propagation::Proceed
    });
    window.add_controller(event_controller);

    window.init_layer_shell();
    window.set_namespace(Some(APP_ID));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.set_exclusive_zone(-1);

    window
}

fn build_restore_checkbox(restore_token: Rc<RefCell<bool>>, config: &Config) -> impl IsA<Widget> {
    let button = CheckButton::builder()
        .css_classes([config.classes.restore_button.as_str()])
        .label("Allow a restore token")
        .active(*restore_token.borrow())
        .build();

    button.connect_toggled(move |btn| {
        *restore_token.borrow_mut() = btn.is_active();
    });

    button
}
