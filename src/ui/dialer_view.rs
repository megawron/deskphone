use gtk4 as gtk;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[allow(dead_code)]
pub struct DialerViewComponents {
    pub container: gtk::Box,
    pub display_label: gtk::Label,
    pub call_btn: gtk::Button,
    pub number_buffer: Rc<RefCell<String>>,
}

pub fn create_dialer_view<F>(on_trigger_call: F) -> DialerViewComponents
where
    F: Fn(String, String) + 'static + Clone,
{
    let container = gtk::Box::new(gtk::Orientation::Vertical, 14);
    container.set_margin_top(20);
    container.set_margin_bottom(20);
    container.set_margin_start(12);
    container.set_margin_end(12);
    container.set_valign(gtk::Align::Center);
    container.set_halign(gtk::Align::Center);

    let number_buffer = Rc::new(RefCell::new(String::new()));

    // 1. Large Display with Backspace
    let display_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    display_box.set_halign(gtk::Align::Center);
    display_box.set_size_request(240, 50);
    display_box.add_css_class("card");
    display_box.set_margin_bottom(12);

    let display_label = gtk::Label::new(Some(&crate::i18n::tr("Enter number")));
    display_label.set_hexpand(true);
    display_label.add_css_class("title-1");
    display_label.add_css_class("dim-label");
    display_label.set_selectable(true);

    // Pre-create Call Button so sensitivity follows buffer validity
    let call_btn = gtk::Button::from_icon_name("call-start-symbolic");
    call_btn.set_size_request(56, 56);
    call_btn.set_halign(gtk::Align::Center);
    call_btn.add_css_class("suggested-action");
    call_btn.add_css_class("circular");
    call_btn.add_css_class("dialer-call-btn");
    call_btn.set_margin_top(10);
    call_btn.set_sensitive(false);
    call_btn.set_tooltip_text(Some(&crate::i18n::tr("Enter number")));

    let backspace_btn = gtk::Button::from_icon_name("edit-clear-symbolic");
    backspace_btn.add_css_class("flat");
    backspace_btn.add_css_class("circular");
    backspace_btn.set_tooltip_text(Some(&crate::i18n::tr("Delete digit (Backspace)")));

    let num_ref_bk = number_buffer.clone();
    let lbl_ref_bk = display_label.clone();
    let call_btn_bk = call_btn.clone();
    backspace_btn.connect_clicked(move |_| {
        let mut buf = num_ref_bk.borrow_mut();
        buf.pop();
        if buf.is_empty() {
            lbl_ref_bk.set_text(&crate::i18n::tr("Enter number"));
            lbl_ref_bk.add_css_class("dim-label");
            call_btn_bk.set_sensitive(false);
        } else {
            lbl_ref_bk.set_text(&buf);
            lbl_ref_bk.remove_css_class("dim-label");
            let valid = buf.chars().any(|c| c.is_ascii_digit()) && buf.len() >= 2;
            call_btn_bk.set_sensitive(valid);
        }
    });

    display_box.append(&display_label);
    display_box.append(&backspace_btn);
    container.append(&display_box);

    // 2. Numeric Keypad Grid
    let grid = gtk::Grid::new();
    grid.set_row_spacing(10);
    grid.set_column_spacing(14);
    grid.set_halign(gtk::Align::Center);

    let keys = [
        ("1", ""), ("2", "ABC"), ("3", "DEF"),
        ("4", "GHI"), ("5", "JKL"), ("6", "MNO"),
        ("7", "PQRS"), ("8", "TUV"), ("9", "WXYZ"),
        ("*", ""), ("0", "+"), ("#", ""),
    ];

    for (index, (digit, sub)) in keys.iter().enumerate() {
        let col = (index % 3) as i32;
        let row = (index / 3) as i32;

        let btn_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        btn_box.set_valign(gtk::Align::Center);
        btn_box.set_halign(gtk::Align::Center);

        let d_lbl = gtk::Label::new(Some(digit));
        d_lbl.add_css_class("title-2");

        btn_box.append(&d_lbl);
        if !sub.is_empty() {
            let s_lbl = gtk::Label::new(Some(sub));
            s_lbl.add_css_class("caption");
            s_lbl.add_css_class("dim-label");
            btn_box.append(&s_lbl);
        }

        let key_btn = gtk::Button::new();
        key_btn.set_child(Some(&btn_box));
        key_btn.set_size_request(60, 48);
        key_btn.add_css_class("circular");

        let digit_str = digit.to_string();
        let num_ref = number_buffer.clone();
        let lbl_ref = display_label.clone();
        let call_btn_key = call_btn.clone();
        key_btn.connect_clicked(move |_| {
            let mut buf = num_ref.borrow_mut();
            buf.push_str(&digit_str);
            lbl_ref.set_text(&buf);
            lbl_ref.remove_css_class("dim-label");
            let valid = buf.chars().any(|c| c.is_ascii_digit()) && buf.len() >= 2;
            call_btn_key.set_sensitive(valid);
        });

        grid.attach(&key_btn, col, row, 1, 1);
    }

    container.append(&grid);

    // 3. Large Circular Green Call Button
    let num_ref_call = number_buffer.clone();
    let on_call_clone = on_trigger_call.clone();
    call_btn.connect_clicked(move |_| {
        let num = num_ref_call.borrow().clone();
        if num.chars().any(|c| c.is_ascii_digit()) && num.len() >= 2 {
            on_call_clone(format!("Nummer: {}", num), num);
        }
    });

    container.append(&call_btn);

    // Keyboard Controller
    let key_controller = gtk::EventControllerKey::new();
    let num_key = number_buffer.clone();
    let lbl_key = display_label.clone();
    let call_btn_kbd = call_btn.clone();
    let on_call_key = on_trigger_call;
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gtk::gdk::Key::Return || keyval == gtk::gdk::Key::KP_Enter {
            let num = num_key.borrow().clone();
            if num.chars().any(|c| c.is_ascii_digit()) && num.len() >= 2 {
                on_call_key(format!("Nummer: {}", num), num);
                return gtk::glib::Propagation::Stop;
            }
        } else if keyval == gtk::gdk::Key::BackSpace {
            let mut buf = num_key.borrow_mut();
            buf.pop();
            if buf.is_empty() {
                lbl_key.set_text("Nummer eingeben");
                lbl_key.add_css_class("dim-label");
                call_btn_kbd.set_sensitive(false);
            } else {
                lbl_key.set_text(&buf);
                lbl_key.remove_css_class("dim-label");
                let valid = buf.chars().any(|c| c.is_ascii_digit()) && buf.len() >= 2;
                call_btn_kbd.set_sensitive(valid);
            }
            return gtk::glib::Propagation::Stop;
        }

        if let Some(ch) = keyval.to_unicode() {
            if ch.is_ascii_digit() || ch == '*' || ch == '#' || ch == '+' {
                let mut buf = num_key.borrow_mut();
                buf.push(ch);
                lbl_key.set_text(&buf);
                lbl_key.remove_css_class("dim-label");
                let valid = buf.chars().any(|c| c.is_ascii_digit()) && buf.len() >= 2;
                call_btn_kbd.set_sensitive(valid);
                return gtk::glib::Propagation::Stop;
            }
        }
        gtk::glib::Propagation::Proceed
    });
    container.add_controller(key_controller);

    DialerViewComponents {
        container,
        display_label,
        call_btn,
        number_buffer,
    }
}

