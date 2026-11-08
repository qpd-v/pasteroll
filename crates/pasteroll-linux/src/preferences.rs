//! New Linux settings shared by the native X11 window and GNOME companion.
use pasteroll_core::{config, json::{self, Json, object, string}, Store};
use pasteroll_core::store::RollSide;
use std::path::Path;

pub fn state(db: &Path, store: &Store) -> Result<String, String> {
    let c = crate::keystore::load_config(db).map_err(|e| e.join("; "))?;
    let rolls = store.rolls().map_err(|e| e.to_string())?;
    Ok(json::stringify(&object(vec![
        ("version", string(env!("CARGO_PKG_VERSION"))),
        ("mode", string(c.mode.key())),
        ("left", Json::Bool(c.rolls.left)), ("right", Json::Bool(c.rolls.right)),
        ("most_used", Json::Bool(c.rolls.most_used)), ("images", Json::Bool(c.rolls.images)),
        ("left_key", string(c.rolls.left_key.map(|k| k.text()).unwrap_or_default())),
        ("right_key", string(c.rolls.right_key.map(|k| k.text()).unwrap_or_default())),
        ("tilt_left", string(c.rolls.tilt_left.key())), ("tilt_right", string(c.rolls.tilt_right.key())),
        ("image_folder", string(c.image_folder)), ("modifier", string(c.advanced_image_modifier.label().to_ascii_lowercase())),
        ("rolls", Json::Array(rolls.iter().map(|r| object(vec![
            ("id", Json::Number(r.id as f64)), ("name", string(&r.name)),
            ("side", string(r.side.label())), ("pos", Json::Number(r.pos as f64)),
        ])).collect())),
    ])))
}

fn text<'a>(j: &'a Json, key: &str) -> Result<&'a str, String> {
    j.get(key).and_then(Json::as_str).ok_or_else(|| format!("Missing {key}"))
}

/// Only explicit UI commands can mutate settings. Clip deletion requires a separate confirmation.
pub fn apply(db: &Path, store: &mut Store, input: &str) -> Result<(), String> {
    if input.len() > 16_384 { return Err("Settings request is too large".into()); }
    let j = json::parse(input)?;
    let command = text(&j, "command")?;
    match command {
        "settings" => {
            let mut saved = crate::keystore::read_config_text(db)?;
            if j.get("left_key").is_some() && j.get("right_key").is_some() {
                for key in ["roll_left_key", "roll_right_key"] {
                    saved = config::set_value(&saved, key, "").map_err(|e| e.join("; "))?;
                }
            }
            for (field, key) in [("mode", "mode"), ("left", "rolls.left_roll"), ("right", "rolls.right_roll"),
                ("most_used", "rolls.most_used"), ("images", "rolls.images"),
                ("left_key", "rolls.roll_left_key"), ("right_key", "rolls.roll_right_key"),
                ("tilt_left", "rolls.tilt_left"), ("tilt_right", "rolls.tilt_right"),
                ("image_folder", "image_folder"), ("modifier", "advanced_image_modifier")] {
                if let Some(value) = j.get(field) {
                    let value = match value { Json::String(s) => s.clone(), Json::Bool(b) => b.to_string(), _ => return Err("Invalid setting".into()) };
                    saved = config::set_value(&saved, key, &value).map_err(|e| e.join("; "))?;
                }
            }
            crate::keystore::write_config_text(db, &saved)
        }
        "add" => store.add_roll(text(&j, "name")?).map(|_| ()).map_err(|e| e.to_string()),
        "rename" | "left" | "right" | "up" | "down" | "remove" => {
            let id = j.get("id").and_then(Json::as_u64).filter(|n| *n <= i64::MAX as u64).ok_or("Invalid roll")? as i64;
            let rolls = store.rolls().map_err(|e| e.to_string())?;
            let r = rolls.iter().find(|r| r.id == id).ok_or("That roll no longer exists")?;
            match command {
                "rename" => store.rename_roll(id, text(&j, "name")?).map_err(|e| e.to_string()),
                "remove" => store.delete_roll(id).map_err(|e| e.to_string()),
                "left" | "right" => {
                    let side = if command == "left" { RollSide::Left } else { RollSide::Right };
                    let pos = rolls.iter().filter(|r| r.side == side).map(|r| r.pos).max().unwrap_or(0) + 1;
                    store.place_roll(id, side, pos).map_err(|e| e.to_string())
                }
                _ => {
                    let side: Vec<_> = rolls.iter().filter(|x| x.side == r.side).collect();
                    let index = side.iter().position(|x| x.id == id).unwrap();
                    let other = if command == "up" { index.checked_sub(1) } else { (index + 1 < side.len()).then_some(index + 1) };
                    if let Some(other) = other {
                        let prev = side[other];
                        store.place_roll(id, r.side, prev.pos).map_err(|e| e.to_string())?;
                        store.place_roll(prev.id, prev.side, r.pos).map_err(|e| e.to_string())?;
                    }
                    Ok(())
                }
            }
        }
        "clear" if j.get("confirmed").and_then(Json::as_bool) == Some(true) => {
            store.clear(true).map(|_| ()).map_err(|e| e.to_string())
        }
        _ => Err("Unknown settings action".into()),
    }
}

/// Compact toolkit-free page, hosted in the existing X11 settings window.
pub struct Editor {
    pub data: Json,
    pub selected: Option<usize>,
    pub entry: String,
    pub focus: u32,
    pub status: String,
    pub confirming: bool,
}

impl Editor {
    pub fn new(state: &str) -> Result<Self, String> {
        Ok(Self { data: json::parse(state)?, selected: None, entry: String::new(), focus: 0, status: String::new(), confirming: false })
    }
    fn value(&self, key: &str) -> String { self.data.get(key).and_then(Json::as_str).unwrap_or("").into() }
    fn set(&mut self, key: &str, value: Json) {
        if let Json::Object(fields) = &mut self.data {
            if let Some(pair) = fields.iter_mut().find(|(k,_)| k == key) { pair.1 = value; }
        }
    }
    pub fn view(&self) -> pasteroll_render::UiView {
        use pasteroll_render::{Chrome, Rect, UiView, Widget, WKind};
        let mut w = Vec::new();
        let mut add = |id,x,y,width,height,label:String,kind| w.push(Widget::new(id,Rect{x,y,w:width,h:height},label,kind));
        add(0,16.,12.,488.,28.,format!("Pasteroll {} — Rolls and appearance",self.value("version")),WKind::Heading);
        add(1,16.,48.,230.,26.,"No Bloat".into(),WKind::Check { checked: self.value("mode") == "no-bloat" });
        for (i,(key,label)) in [("left","LEFT"),("right","RIGHT"),("most_used","Most used"),("images","Images")].iter().enumerate() {
            add(10+i as u32,16.+i as f32*122.,86.,120.,26.,label.to_string(),WKind::Check { checked: self.data.get(key).and_then(Json::as_bool).unwrap_or(false) });
        }
        for (i,(key,label)) in [("left_key","Roll left"),("right_key","Roll right"),("image_folder","Picture folder")].iter().enumerate() {
            let y=124.+i as f32*38.;
            add(0,16.,y,120.,28.,label.to_string(),WKind::Label);
            add(20+i as u32,140.,y,364.,28.,self.value(key),WKind::Entry { placeholder:"".into(),focused:self.focus==20+i as u32 });
        }
        for (i,key) in ["tilt_left","tilt_right"].iter().enumerate() {
            add(30+i as u32,16.+i as f32*246.,242.,242.,30.,format!("Tilt {}: {}",if i==0 {"left"} else {"right"},self.value(key)),WKind::Button{primary:false});
        }
        let rows=self.data.get("rolls").and_then(Json::as_array).unwrap_or(&[]);
        add(40,16.,286.,488.,196.,String::new(),WKind::List { items:rows.iter().map(|r| format!("{} · {}",text(r,"name").unwrap_or(""),text(r,"side").unwrap_or(""))).collect(),selected:self.selected,top:0 });
        add(41,16.,494.,290.,28.,self.entry.clone(),WKind::Entry { placeholder:"Roll name".into(),focused:self.focus==41 });
        for (id,x,label) in [(42,314.,"New"),(43,410.,"Rename")] { add(id,x,494.,94.,28.,label.into(),WKind::Button{primary:false}); }
        for (i,label) in ["Left","Right","Up","Down","Remove"].iter().enumerate() { add(44+i as u32,16.+i as f32*98.,534.,94.,28.,label.to_string(),WKind::Button{primary:false}); }
        add(0,16.,574.,488.,50.,"Removing a roll returns its clips to Main. Agent sharing is managed with pasteroll brain and pasteroll mcp-config.".into(),WKind::Note);
        add(50,16.,636.,240.,30.,if self.confirming {"Confirm clear (keeps favorites)"} else {"Clear history…"}.into(),WKind::Button{primary:false});
        add(0,16.,678.,488.,52.,self.status.clone(),WKind::Note);
        add(51,314.,744.,94.,32.,"Save".into(),WKind::Button{primary:true});
        add(52,410.,744.,94.,32.,"Back".into(),WKind::Button{primary:false});
        UiView {size:(520.,792.),chrome:Chrome::Window,widgets:w,hover:None}
    }
    pub fn click(&mut self, x:f32,y:f32) -> Option<String> {
        use pasteroll_render::{hit,list_row_at};
        let view=self.view(); let widget=hit(&view,x,y)?; let id=widget.id;
        self.focus=if matches!(id,20..=22|41) {id} else {0};
        self.status.clear();
        if id!=50 { self.confirming=false; }
        match id {
            1 => self.set("mode",string(if self.value("mode")=="no-bloat" {"complete"} else {"no-bloat"})),
            10..=13 => { let key=["left","right","most_used","images"][(id-10) as usize]; let value=self.data.get(key).and_then(Json::as_bool).unwrap_or(false); self.set(key,Json::Bool(!value)); }
            30|31 => { let key=if id==30 {"tilt_left"} else {"tilt_right"}; let value=match self.value(key).as_str() {"roll-left"=>"roll-right","roll-right"=>"none",_=>"roll-left"}; self.set(key,string(value)); }
            40 => {self.selected=list_row_at(widget,y); if let Some(r)=self.selected.and_then(|i|self.data.get("rolls")?.as_array()?.get(i)) { self.entry=text(r,"name").unwrap_or("").into(); } }
            42..=48 => {
                let command=["add","rename","left","right","up","down","remove"][(id-42) as usize];
                let roll=self.selected.and_then(|i|self.data.get("rolls")?.as_array()?.get(i)).and_then(|r|r.get("id")).cloned().unwrap_or(Json::Null);
                return Some(json::stringify(&object(vec![("command",string(command)),("name",string(&self.entry)),("id",roll)])));
            }
            50 => { if self.confirming { self.confirming=false; return Some("{\"command\":\"clear\",\"confirmed\":true}".into()); } self.confirming=true; }
            51 => { let mut j=self.data.clone(); if let Json::Object(v)=&mut j { v.push(("command".into(),string("settings"))); } return Some(json::stringify(&j)); }
            _=>{}
        }
        None
    }
    pub fn key(&mut self, key:pasteroll_app::Key) {
        let field=match self.focus {20=>"left_key",21=>"right_key",22=>"image_folder",41=>"entry",_=>return};
        let mut value=if field=="entry" {self.entry.clone()} else {self.value(field)};
        match key { pasteroll_app::Key::Char(c) if value.len()<4096 => value.push(c), pasteroll_app::Key::Backspace=>{value.pop();},pasteroll_app::Key::CtrlA=>value.clear(),_=>{} }
        if field=="entry" {self.entry=value;} else {self.set(field,string(value));}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_preferences_do_not_partially_save_and_roll_commands_preserve_clips() {
        let dir=std::env::temp_dir().join(format!("pasteroll-preferences-{}",std::process::id()));
        std::fs::create_dir_all(&dir).unwrap(); let db=dir.join("history.db");
        let mut store=crate::keystore::open_store(&db).unwrap();
        let original=crate::keystore::read_config_text(&db).unwrap();
        assert!(apply(&db,&mut store,r#"{"command":"settings","left":false,"right_key":"ctrl+k"}"#).is_err());
        assert_eq!(crate::keystore::read_config_text(&db).unwrap(),original);
        apply(&db,&mut store,r#"{"command":"settings","left":false,"left_key":"ctrl+right","right_key":"ctrl+left"}"#).unwrap();
        let saved=crate::keystore::load_config(&db).unwrap();
        assert!(!saved.rolls.left);
        assert_eq!(saved.rolls.left_key.unwrap().text(),"ctrl+right");
        apply(&db,&mut store,r#"{"command":"add","name":"Work"}"#).unwrap();
        let roll=store.rolls().unwrap()[0].id;
        let clip=store.add_text("synthetic upgrade fixture",1800000000).unwrap();
        store.set_clip_roll(clip,Some(roll)).unwrap();
        apply(&db,&mut store,&format!(r#"{{"command":"remove","id":{roll}}}"#)).unwrap();
        assert_eq!(store.roll_of(clip).unwrap(),None);
        assert_eq!(store.main_len().unwrap(),1);
        assert!(apply(&db,&mut store,r#"{"command":"clear"}"#).is_err());
        drop(store); std::fs::remove_dir_all(dir).unwrap();
    }
}
