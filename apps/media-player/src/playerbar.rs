//! The bar under the library pages: a small live picture of what's playing,
//! transport and seek, which subtitles and sound track are on, the sound
//! button (everything about sound in one place), volume and the way to the
//! player. It hides on the player page, whose controls float over the video.

use crate::library::Kind;
use crate::library::art::Art;
use crate::player::{self, Event, State, TrackInfo, TrackKind, video};
use crate::sections::now_playing;
use crate::seekbar::SeekBar;
use crate::{prefs, widgets, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;

thread_local! {
    static NARROW_PARTS: RefCell<Vec<gtk::Widget>> = const { RefCell::new(Vec::new()) };
    /// Shown only in narrow windows, in place of the parts above.
    static NARROW_ONLY: RefCell<Vec<gtk::Widget>> = const { RefCell::new(Vec::new()) };
    /// The track chips, shown only in wide windows and only with something to say.
    static CHIPS: RefCell<Vec<gtk::MenuButton>> = const { RefCell::new(Vec::new()) };
    static NARROW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn set_narrow(narrow: bool) {
    NARROW.with(|n| n.set(narrow));
    NARROW_PARTS.with(|p| {
        for w in p.borrow().iter() {
            w.set_visible(!narrow);
        }
    });
    NARROW_ONLY.with(|p| {
        for w in p.borrow().iter() {
            w.set_visible(narrow);
        }
    });
    refresh_chips();
}

fn volume_icon(v: f64, muted: bool) -> &'static str {
    if muted || v <= 0.001 {
        "audio-volume-muted-symbolic"
    } else if v < 0.34 {
        "audio-volume-low-symbolic"
    } else if v < 0.67 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    }
}

/// A track's name for a chip: "English", "English 5.1", "Commentary".
fn chip_text(t: &TrackInfo) -> String {
    let lang = player::language_name(&t.lang);
    let mut name = match (lang.is_empty(), t.title.is_empty()) {
        (false, _) => lang,
        (true, false) => t.title.clone(),
        (true, true) => format!("Track {}", t.id),
    };
    if t.kind == TrackKind::Audio {
        match t.channels {
            6 => name.push_str(" 5.1"),
            8 => name.push_str(" 7.1"),
            _ => {}
        }
    }
    name
}

/// A menu button, filled fresh each time it opens.
fn menu_button(fill: impl Fn(&gtk::Box, &gtk::Popover) + 'static) -> gtk::MenuButton {
    let mb = gtk::MenuButton::new();
    mb.set_focus_on_click(false);
    mb.set_valign(gtk::Align::Center);
    mb.set_direction(gtk::ArrowType::Up);
    let pop = gtk::Popover::new();
    pop.add_css_class("menu-popover");
    pop.set_child(Some(&widgets::vbox(1)));
    mb.set_popover(Some(&pop));
    nyx_playback::controls::fill_menu(&mb, fill);
    mb
}

/// A chip saying which track is on, opening the menu to change it.
fn track_chip(icon: &str, tooltip: &str, fill: impl Fn(&gtk::Box, &gtk::Popover) + 'static) -> (gtk::MenuButton, gtk::Label) {
    let mb = menu_button(fill);
    mb.add_css_class("track-chip");
    mb.set_tooltip_text(Some(tooltip));
    let face = widgets::hbox(6);
    face.append(&gtk::Image::from_icon_name(icon));
    let label = gtk::Label::new(None);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(14);
    face.append(&label);
    mb.set_child(Some(&face));
    (mb, label)
}

/// Show the chips that have something to say, in a wide window.
fn refresh_chips() {
    let narrow = NARROW.with(|n| n.get());
    CHIPS.with(|c| {
        for chip in c.borrow().iter() {
            let has = chip.child().and_then(|f| f.last_child()).and_downcast::<gtk::Label>().is_some_and(|l| !l.text().is_empty());
            chip.set_visible(!narrow && has);
        }
    });
}

pub fn build() -> gtk::Box {
    let bar = widgets::hbox(16);
    bar.add_css_class("player-bar");

    // ----- Now playing -----
    let info = widgets::hbox(12);
    info.add_css_class("player-info");
    info.set_size_request(240, -1);
    // The art sizes the thumbnail; the video sits over it as an overlay, which
    // isn't measured, so a full-size frame can't stretch the bar.
    let thumb = gtk::Overlay::new();
    thumb.add_css_class("mini-video");
    thumb.set_valign(gtk::Align::Center);
    thumb.set_overflow(gtk::Overflow::Hidden);
    let art = Art::wide(96);
    thumb.set_child(Some(&art.root));
    let picture = gtk::Picture::for_paintable(&video::paintable());
    picture.add_css_class("mini-video");
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_visible(false);
    thumb.add_overlay(&picture);
    info.append(&thumb);
    let text = widgets::vbox(2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let title = widgets::label("Nothing playing", "player-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(26);
    title.set_xalign(0.0);
    let sub = widgets::label("", "player-artist");
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    sub.set_max_width_chars(26);
    sub.set_xalign(0.0);
    text.append(&title);
    text.append(&sub);
    info.append(&text);
    let click = gtk::GestureClick::new();
    click.connect_released(|_, _, _, _| window::navigate("now-playing"));
    info.add_controller(click);
    nyx_ui::menu::on_right_click(&info, |w, x, y| {
        nyx_ui::menu::popup_at(
            w,
            x,
            y,
            vec![
                nyx_ui::menu::item("Open the player", || window::navigate("now-playing")).icon("view-fullscreen-symbolic"),
                nyx_ui::menu::item("Close the video", player::close).icon("window-close-symbolic"),
            ],
        );
    });
    info.set_cursor_from_name(Some("pointer"));
    info.set_tooltip_text(Some("Open the player (right-click to close the video)"));
    bar.append(&info);

    // ----- Transport + seek -----
    let center = widgets::vbox(2);
    center.set_hexpand(true);
    center.set_size_request(260, -1);
    center.set_valign(gtk::Align::Center);
    let controls = widgets::hbox(6);
    controls.set_halign(gtk::Align::Center);
    let skip = prefs::get().skip_short;
    let back = widgets::icon_button("media-seek-backward-symbolic", &format!("Back {skip:.0} s (←)"));
    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    play.add_css_class("play-button");
    play.set_tooltip_text(Some("Play (Space)"));
    let fwd = widgets::icon_button("media-seek-forward-symbolic", &format!("Forward {skip:.0} s (→)"));
    let next = widgets::icon_button("media-skip-forward-symbolic", "Next (Ctrl+→)");
    for w in [back.upcast_ref::<gtk::Widget>(), play.upcast_ref(), fwd.upcast_ref(), next.upcast_ref()] {
        w.set_focus_on_click(false);
        controls.append(w);
    }
    center.append(&controls);
    center.append(&SeekBar::new().root);
    bar.append(&center);

    // ----- Tracks, sound, volume, the player -----
    let right = widgets::hbox(8);
    right.set_valign(gtk::Align::Center);
    right.set_halign(gtk::Align::End);
    let (subs, subs_label) = track_chip("nmp-subtitles-symbolic", "Subtitles (S)", now_playing::subtitle_menu);
    let (audio, audio_label) = track_chip("audio-volume-high-symbolic", "Sound track (A)", now_playing::audio_menu);
    right.append(&subs);
    right.append(&audio);
    CHIPS.with(|c| c.borrow_mut().extend([subs.clone(), audio.clone()]));
    let sound = menu_button(now_playing::sound_menu);
    sound.set_icon_name("nmp-equalizer-symbolic");
    sound.add_css_class("icon-button");
    sound.set_tooltip_text(Some("Sound: track, delay, night mode, boost, equalizer"));
    right.append(&sound);
    let p = prefs::get();
    let mute = widgets::icon_button(volume_icon(p.volume, p.muted), "Mute (M)");
    mute.set_focus_on_click(false);
    right.append(&mute);
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, player::max_volume(), 0.01);
    volume.set_draw_value(false);
    volume.set_value(p.volume);
    volume.set_size_request(100, -1);
    volume.add_css_class("volume");
    volume.set_focus_on_click(false);
    volume.set_tooltip_text(Some(&format!("{}%", (p.volume * 100.0).round())));
    right.append(&volume);
    // Narrow windows: one button that opens the volume.
    let compact = gtk::MenuButton::new();
    compact.set_icon_name(volume_icon(p.volume, p.muted));
    compact.set_tooltip_text(Some("Volume"));
    compact.add_css_class("icon-button");
    compact.set_visible(false);
    let pop = gtk::Popover::new();
    let col = widgets::vbox(8);
    col.add_css_class("volume-popover");
    let vertical = gtk::Scale::with_range(gtk::Orientation::Vertical, 0.0, player::max_volume(), 0.01);
    vertical.set_inverted(true);
    vertical.set_draw_value(false);
    vertical.set_value(p.volume);
    vertical.set_size_request(-1, 140);
    vertical.add_css_class("volume");
    vertical.set_halign(gtk::Align::Center);
    vertical.connect_change_value(|_, _, v| {
        player::set_volume(v);
        if prefs::get().muted && v > 0.0 {
            player::set_muted(false);
        }
        glib::Propagation::Proceed
    });
    let pop_mute = widgets::icon_button(volume_icon(p.volume, p.muted), "Mute");
    pop_mute.connect_clicked(|_| player::set_muted(!prefs::get().muted));
    col.append(&vertical);
    col.append(&pop_mute);
    pop.set_child(Some(&col));
    compact.set_popover(Some(&pop));
    right.append(&compact);
    let expand = widgets::icon_button("view-fullscreen-symbolic", "Open the player");
    expand.set_focus_on_click(false);
    expand.connect_clicked(|_| window::navigate("now-playing"));
    right.append(&expand);
    bar.append(&right);
    NARROW_PARTS.with(|n| n.borrow_mut().extend([volume.clone().upcast(), mute.clone().upcast()]));
    NARROW_ONLY.with(|n| n.borrow_mut().push(compact.clone().upcast()));

    // ----- Wiring -----
    play.connect_clicked(|_| player::toggle());
    back.connect_clicked(|_| player::seek_by(-prefs::get().skip_short));
    fwd.connect_clicked(|_| player::seek_by(prefs::get().skip_short));
    next.connect_clicked(|_| player::next());
    mute.connect_clicked(|_| player::set_muted(!prefs::get().muted));
    volume.connect_change_value(|s, _, v| {
        let v = v.clamp(0.0, player::max_volume());
        player::set_volume(v);
        if prefs::get().muted && v > 0.0 {
            player::set_muted(false);
        }
        s.set_tooltip_text(Some(&format!("{}%", (v * 100.0).round())));
        glib::Propagation::Proceed
    });
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll(|_, _, dy| {
        player::set_volume(prefs::get().volume - dy * 0.05);
        glib::Propagation::Stop
    });
    mute.add_controller(scroll);

    let refresh = {
        let (picture, art, title, sub, play, mute, volume, back, fwd, next) = (
            picture.clone(),
            art.clone(),
            title.clone(),
            sub.clone(),
            play.clone(),
            mute.clone(),
            volume.clone(),
            back.clone(),
            fwd.clone(),
            next.clone(),
        );
        let (compact, pop_mute, vertical) = (compact.clone(), pop_mute.clone(), vertical.clone());
        move |e: Event| match e {
            Event::Track | Event::Video => {
                match player::current() {
                    Some(v) => {
                        let (t, s) = match v.kind {
                            Kind::Episode => (v.show.clone(), v.episode_line()),
                            _ => (v.title.clone(), v.year.map(|y| y.to_string()).unwrap_or_default()),
                        };
                        title.set_text(&t);
                        sub.set_text(&s);
                        title.set_tooltip_text(Some(&v.label()));
                        art.set_key(v.wide_art());
                    }
                    None => {
                        title.set_text("Nothing playing");
                        sub.set_text("");
                        art.set_key("");
                    }
                }
                let live = player::has_video() || video::paintable().has_frame();
                picture.set_visible(live);
                let loaded = player::current().is_some();
                back.set_sensitive(loaded);
                fwd.set_sensitive(loaded);
                next.set_sensitive(player::can_next());
            }
            Event::Tracks => {
                let loaded = player::current().is_some();
                let subs = player::tracks(TrackKind::Sub);
                subs_label.set_text(&match subs.iter().find(|t| t.selected) {
                    Some(t) => chip_text(t),
                    None if loaded => "Off".into(),
                    None => String::new(),
                });
                let tracks = player::tracks(TrackKind::Audio);
                audio_label.set_text(&tracks.iter().find(|t| t.selected).map(chip_text).unwrap_or_default());
                refresh_chips();
            }
            Event::State => {
                let playing = player::state() == State::Playing;
                play.set_icon_name(if playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
                play.set_tooltip_text(Some(if playing { "Pause (Space)" } else { "Play (Space)" }));
            }
            Event::Queue => next.set_sensitive(player::can_next()),
            Event::Options => {
                let p = prefs::get();
                mute.set_icon_name(volume_icon(p.volume, p.muted));
                compact.set_icon_name(volume_icon(p.volume, p.muted));
                pop_mute.set_icon_name(volume_icon(p.volume, p.muted));
                if (vertical.adjustment().upper() - player::max_volume()).abs() > 0.001 {
                    vertical.set_range(0.0, player::max_volume());
                }
                if (vertical.value() - p.volume).abs() > 0.005 {
                    vertical.set_value(p.volume);
                }
                mute.set_tooltip_text(Some(if p.muted { "Unmute (M)" } else { "Mute (M)" }));
                if (volume.adjustment().upper() - player::max_volume()).abs() > 0.001 {
                    volume.set_range(0.0, player::max_volume());
                }
                if (volume.value() - p.volume).abs() > 0.005 {
                    volume.set_value(p.volume);
                }
            }
            _ => {}
        }
    };
    for e in [Event::Track, Event::Tracks, Event::State, Event::Queue, Event::Options] {
        refresh(e);
    }
    player::subscribe(&bar, refresh);
    video::paintable().connect_invalidate_contents(move |p| {
        if p.has_frame() && !picture.is_visible() {
            picture.set_visible(true);
        }
    });
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(kind: TrackKind, lang: &str, title: &str, channels: i64) -> TrackInfo {
        TrackInfo {
            kind,
            id: 2,
            title: title.into(),
            lang: lang.into(),
            codec: String::new(),
            channels,
            external: false,
            forced: false,
            selected: true,
        }
    }

    #[test]
    fn names_tracks_briefly() {
        assert_eq!(chip_text(&track(TrackKind::Audio, "eng", "", 6)), "English 5.1");
        assert_eq!(chip_text(&track(TrackKind::Audio, "eng", "", 2)), "English");
        assert_eq!(chip_text(&track(TrackKind::Sub, "", "Commentary", 0)), "Commentary");
        assert_eq!(chip_text(&track(TrackKind::Sub, "", "", 0)), "Track 2");
    }
}
