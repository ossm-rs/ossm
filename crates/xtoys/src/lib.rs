#![no_std]

//! XToys protocol adapter.
//!
//! Parsing and XToys-specific behavior live here. The crate emits generic
//! control operations through `control-interface`; it does not modify or
//! depend on pattern-engine internals.

use control_interface::ControlSender;
use heapless::String;

pub const MAX_MESSAGE_LEN: usize = 240;

#[derive(Debug, Clone, Copy)]
pub enum Action<'a> {
    Connected,
    Home,
    SetPattern(usize),
    Pause,
    Resume,
    Stop,
    SetSpeed(f64),
    SetDepth(f64),
    SetStroke(f64),
    SetSensation(f64),
    Move{position:f64,time_ms:u32,replace:bool},
    PositionSeed,
    Disable,
    Version,
    ConfigureBluetooth,
    GetPatternList,
    Setup,
    Retract,
    Extend,
    StartStreaming,
    Unsupported(&'a str),
}

fn value<'a>(o:&'a str,key:&str)->Option<&'a str>{
    let mut k:String<48>=String::new(); use core::fmt::Write as _; write!(k,"\"{}\"",key).ok()?;
    let s=o.find(k.as_str())?; let a=&o[s+k.len()..]; let c=a.find(':')?; Some(a[c+1..].trim_start())
}
fn string<'a>(o:&'a str,key:&str)->Option<&'a str>{let v=value(o,key)?.strip_prefix('"')?;Some(&v[..v.find('"')?])}
fn number(o:&str,key:&str)->Option<f64>{let v=value(o,key)?;let e=v.find(|c:char|!(c.is_ascii_digit()||matches!(c,'-'|'+'|'.'))).unwrap_or(v.len());v[..e].parse().ok()}
fn boolean(o:&str,key:&str)->bool{value(o,key).map(|v|v.starts_with("true")).unwrap_or(false)}

pub fn parse_object(object:&str)->Option<Action<'_>>{
    let a=string(object,"action")?;
    Some(match a {
        "connected"=>Action::Connected,
        "home"=>Action::Home,
        "setPattern"=>Action::SetPattern(number(object,"pattern").unwrap_or(0.0).max(0.0) as usize),
        "pause"=>Action::Pause,"resume"=>Action::Resume,"stop"=>Action::Stop,
        "setSpeed"=>Action::SetSpeed(number(object,"speed").unwrap_or(0.0).clamp(0.0,100.0)/100.0),
        "setDepth"=>Action::SetDepth(number(object,"depth").unwrap_or(0.0).clamp(0.0,100.0)/100.0),
        "setStroke"=>Action::SetStroke(number(object,"stroke").unwrap_or(0.0).clamp(0.0,100.0)/100.0),
        "setSensation"=>Action::SetSensation(number(object,"sensation").unwrap_or(0.0).clamp(-100.0,100.0)/100.0),
        "move"=>{
            if value(object,"time").map(|v|v.starts_with("null")).unwrap_or(false){Action::PositionSeed}
            else {Action::Move{position:number(object,"position").unwrap_or(0.0).clamp(0.0,100.0)/100.0,time_ms:number(object,"time").unwrap_or(0.0).clamp(0.0,u32::MAX as f64) as u32,replace:boolean(object,"replace")}}
        },
        "disable"=>Action::Disable,"version"=>Action::Version,"configureBluetooth"=>Action::ConfigureBluetooth,
        "getPatternList"=>Action::GetPatternList,"setup"=>Action::Setup,"retract"=>Action::Retract,"extend"=>Action::Extend,
        "startStreaming"=>Action::StartStreaming,
        other=>Action::Unsupported(other),
    })
}

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum Reply { None, HomePending, Version, ConfigureBluetoothOk, PatternList }

pub async fn apply(action:Action<'_>, control:&ControlSender)->Reply{
    log::info!("XToys action {:?}",action);
    match action {
        Action::Connected|Action::Setup=>control.stop(),
        Action::Home=>{control.home();return Reply::HomePending},
        Action::SetPattern(i)=>control.play(i),Action::Pause=>control.pause(),Action::Resume=>control.resume(),Action::Stop=>control.stop(),
        Action::SetSpeed(v)=>control.set_speed(v),Action::SetDepth(v)=>control.set_depth(v),Action::SetStroke(v)=>control.set_stroke(v),Action::SetSensation(v)=>control.set_sensation(v),
        Action::Move{position,time_ms,..}=>control.direct_move(position,time_ms).await,
        Action::PositionSeed|Action::StartStreaming=>{},
        Action::Disable=>{control.stop();let _=control.disable_motion().await;},
        Action::Retract=>control.retract().await,Action::Extend=>control.extend().await,
        Action::Version=>return Reply::Version,Action::ConfigureBluetooth=>return Reply::ConfigureBluetoothOk,Action::GetPatternList=>return Reply::PatternList,
        Action::Unsupported(name)=>log::warn!("Unsupported XToys action {}",name),
    }
    Reply::None
}

pub fn for_each_object(message:&str,mut f:impl FnMut(&str)){
    let mut depth=0usize;let mut start=None;
    for (i,c) in message.char_indices(){match c{'{'=>{if depth==0{start=Some(i)}depth+=1},'}'=>{if depth>0{depth-=1;if depth==0{if let Some(s)=start.take(){f(&message[s..=i])}}}},_=>{}}}
}

pub struct ObjectIter<'a>{message:&'a str,scan:usize}
impl<'a> ObjectIter<'a>{pub fn new(message:&'a str)->Self{Self{message,scan:0}}}
impl<'a> Iterator for ObjectIter<'a>{type Item=&'a str;fn next(&mut self)->Option<Self::Item>{
    let bytes=self.message.as_bytes();let mut depth=0usize;let mut start=None;let mut i=self.scan;
    while i<bytes.len(){match bytes[i]{b'{'=>{if depth==0{start=Some(i)}depth+=1},b'}'=>{if depth>0{depth-=1;if depth==0{let s=start?;self.scan=i+1;return Some(&self.message[s..=i])}}},_=>{}};i+=1}
    self.scan=bytes.len();None
}}
