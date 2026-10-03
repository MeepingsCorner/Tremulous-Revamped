//! GDExtension entry point: registers `TremGame` with Godot.
use godot::prelude::*;

mod actors;
mod assets;
mod audio;
mod bspview;
mod cinematic;
mod cgcmds;
mod clientcmd;
mod cmd;
mod mapents;
mod marks;
mod menumodel;
mod menus;
mod md3;
mod models;
mod game;
mod geom;
mod hostfs;
mod hud;
mod input;
mod keys;
mod vidsnd;
mod console;
mod devcmds;
mod joystick;
mod misc;
mod userinfo;
mod view;
mod effects;
mod particles;
mod trails;
mod voipsnd; // wt-b VoIP
mod avicap; // wt-b video capture

struct TremExtension;

#[gdextension]
unsafe impl ExtensionLibrary for TremExtension {}
