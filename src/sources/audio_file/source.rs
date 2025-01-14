//! Implementation of [`AudioSource`] for an audio file on disk, loaded either fully in memory,
//! or by streaming the file directly from disk.

use crate::backend::AudioBackend;
use crate::prelude::{AudioFileError, AudioFileSettings, AudioSource};
use crate::sources::audio_file;
use bevy::asset::Asset;
use bevy::prelude::TypePath;
use bevy::utils::error;
use kira::manager::error::PlaySoundError;
use kira::manager::AudioManager;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle, StreamingSoundSettings};
use kira::sound::{FromFileError, PlaybackRate, PlaybackState, Region};
use kira::tween::{Tween, Value};
use kira::{CommandError, OutputDestination, Volume};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

/// Bevy [`Asset`] implementation that wraps audio data for [`kira`].
///
/// Streaming audio data is currently not possible over the internet, so when targeting the web,
/// all audio sources need to be [`Static`](Self::Static).
#[derive(Asset, Clone, TypePath)]
pub enum AudioFile {
    /// Static audio data, fully loaded in memory.
    Static(Arc<[u8]>, StaticSoundSettings),
    /// Streaming audio data, pointing to a path on disk and loaded on demand.
    Streaming {
        /// Path to the audio file being read
        path: PathBuf,
        /// Settings for the streaming audio file
        settings: StreamingSoundSettings,
    },
}

impl AudioSource for AudioFile {
    type Error = PlaySoundError<AudioFileError>;
    type Handle = AudioFileHandle;
    type Settings = AudioFileSettings;

    fn create_handle(
        &self,
        manager: &mut AudioManager<AudioBackend>,
        settings: &Self::Settings,
        output_destination: OutputDestination,
    ) -> Result<Self::Handle, Self::Error> {
        let start_paused = settings.start_paused;
        match self {
            Self::Static(data, kira_settings) => {
                let settings = (*kira_settings)
                    .output_destination(output_destination)
                    .volume(settings.volume)
                    .panning(settings.panning)
                    .loop_region(settings.loop_region)
                    .reverse(settings.reverse)
                    .playback_region(settings.play_region);
                let static_data = StaticSoundData::from_cursor(Cursor::new(data.clone()), settings)
                    .map_err(|err| {
                        PlaySoundError::IntoSoundError(AudioFileError::FromFileError(err))
                    })?;
                manager
                    .play(static_data)
                    .map_err(audio_file::play_sound_error_transmute)
                    .map(|mut handle| {
                        if start_paused {
                            error(handle.pause(Tween::default()));
                        }
                        handle
                    })
                    .map(RawAudioHandleImpl::Static)
                    .map(AudioFileHandle)
            }
            Self::Streaming {
                path,
                settings: kira_settings,
            } => {
                let settings = (*kira_settings)
                    .output_destination(output_destination)
                    .volume(settings.volume)
                    .panning(settings.panning)
                    .loop_region(settings.loop_region)
                    .playback_region(settings.play_region);
                let streaming_sound_data =
                    StreamingSoundData::from_file(path, settings).map_err(|err| {
                        PlaySoundError::IntoSoundError(AudioFileError::FromFileError(err))
                    })?;
                manager
                    .play(streaming_sound_data)
                    .map_err(audio_file::play_sound_error_cast)
                    .map(|mut handle| {
                        if start_paused {
                            error(handle.pause(Tween::default()));
                        }
                        handle
                    })
                    .map(RawAudioHandleImpl::Streaming)
                    .map(AudioFileHandle)
            }
        }
    }
}

/// Handle to an existing audio file. Access this component in your systems to manipulate the
/// audio in real time (see the `spatial` example to see how to do so).
pub struct AudioFileHandle(RawAudioHandleImpl);

macro_rules! defer_call {
    (fn $name:ident(&self $(, $argname:ident: $argtype:ty)*) -> $ret:ty) => {
        defer_call!(fn $name :: $name(&self $(, $argname: $argtype)*) -> $ret);
    };
    // Don't know how to parametrize the `mut` and be able to factor these two into one variant
    (fn $name:ident :: $fnname:ident(&self $(, $argname:ident: $argtype:ty)*) -> $ret:ty) => {
       /// Forwarded call to [`StaticSoundHandle`] or [`StreamingSoundHandle`].
       ///
       /// Note: Documentation cannot be provided directly due to limitations with docs in macros.
       pub fn $fnname(&self, $($argname: $argtype),*) -> $ret {
            match self {
                Self(RawAudioHandleImpl::Static(handle)) => handle.$name($($argname),*),
                Self(RawAudioHandleImpl::Streaming(handle)) => handle.$name($($argname),*),
            }
        }
    };
    (fn $name:ident(&mut self $(, $argname:ident: $argtype:ty)*) -> $ret:ty) => {
       /// Forwarded call to [`StaticSoundHandle`] or [`StreamingSoundHandle`].
       ///
       /// Note: Documentation cannot be provided directly due to limitations with docs in macros.
        pub fn $name(&mut self, $($argname: $argtype),*) -> $ret {
            match self {
                Self(RawAudioHandleImpl::Static(handle)) => handle.$name($($argname),*),
                Self(RawAudioHandleImpl::Streaming(handle)) => handle.$name($($argname),*),
            }
        }
    };
}

impl AudioFileHandle {
    defer_call!(fn state :: playback_state(&self) -> PlaybackState);
    defer_call!(fn position(&self) -> f64);
    defer_call!(fn set_playback_rate(&mut self, rate: impl Into<Value<PlaybackRate>>, tween: Tween) -> Result<(), CommandError>);
    defer_call!(fn set_panning(&mut self, panning: impl Into<Value<f64>>, tween: Tween) -> Result<(), CommandError>);
    defer_call!(fn set_playback_region(&mut self, region: impl Into<Region>) -> Result<(), CommandError>);
    defer_call!(fn set_loop_region(&mut self, region: impl Into<Region>) -> Result<(), CommandError>);
    defer_call!(fn set_volume(&mut self, volume: impl Into<Value<Volume>>, tween: Tween) -> Result<(), CommandError>);
    defer_call!(fn pause(&mut self, tween: Tween) -> Result<(), CommandError>);
    defer_call!(fn resume(&mut self, tween: Tween) -> Result<(), CommandError>);
    defer_call!(fn stop(&mut self, tween: Tween) -> Result<(), CommandError>);
    defer_call!(fn seek_to(&mut self, position: f64) -> Result<(), CommandError>);
    defer_call!(fn seek_by(&mut self, amount: f64) -> Result<(), CommandError>);
}

/// Enum of the possible sound handles that [`kira`] returns
enum RawAudioHandleImpl {
    Static(StaticSoundHandle),
    Streaming(StreamingSoundHandle<FromFileError>),
}
