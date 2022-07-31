use std::mem::ManuallyDrop;

use bevy::{
    asset::{Asset, Handle as BevyHandle, HandleId},
    prelude::{Assets, Deref, DerefMut, Res, ResMut},
    reflect::TypeUuid,
    tasks::AsyncComputeTaskPool,
    utils::HashMap,
};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleRate,
};
use oddio::{Handle as OddioHandle, Mixer, Signal, SplitSignal, Stop};

use crate::{Audio, Stereo, ToSignal};

/// Used internally in handling audio output.
pub struct AudioOutput {
    mixer_handle: OddioHandle<Mixer<Stereo>>,
}

impl AudioOutput {
    fn play<S>(&mut self, signal: SplitSignal<S::Signal>) -> AudioSink<S>
    where
        S: ToSignal + Asset,
        S::Signal: Signal<Frame = Stereo> + Send,
    {
        AudioSink(ManuallyDrop::new(self.mixer_handle.control().play(signal)))
    }
}

impl Default for AudioOutput {
    fn default() -> Self {
        let task_pool = AsyncComputeTaskPool::get();
        let (mixer_handle, mixer) = oddio::split(oddio::Mixer::new());

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("No default output device available.");
        let sample_rate = device
            .default_output_config()
            .expect("Cannot get default output config.")
            .sample_rate();

        task_pool.spawn(play(mixer, device, sample_rate)).detach();

        Self { mixer_handle }
    }
}

#[allow(clippy::unused_async)]
async fn play(mixer: SplitSignal<Mixer<Stereo>>, device: Device, sample_rate: SampleRate) {
    let config = cpal::StreamConfig {
        channels: 2,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let stream = device
        .build_output_stream(
            &config,
            move |out_flat: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let out_stereo: &mut [Stereo] = oddio::frame_stereo(out_flat);
                oddio::run(&mixer, sample_rate.0, out_stereo);
            },
            move |err| bevy::utils::tracing::error!("Error in cpal: {err:?}"),
        )
        .expect("Cannot build output stream.");
    stream.play().expect("Cannot play stream.");
}

/// System to play queued audio in [`Audio`].
#[allow(clippy::needless_pass_by_value, clippy::missing_panics_doc)]
pub fn play_queued_audio<Source>(
    mut audio_output: ResMut<AudioOutput>,
    audio: Res<Audio<Source>>,
    sources: Res<Assets<Source>>,
    mut sink_assets: ResMut<Assets<AudioSink<Source>>>,
    mut sinks: ResMut<AudioSinks<Source>>,
    mut handle_assets: ResMut<Assets<AudioHandle<Source>>>,
    mut handles: ResMut<AudioHandles<Source>>,
) where
    Source: ToSignal + Asset + Send,
    Source::Signal: Signal<Frame = Stereo> + Send,
{
    let mut queue = audio.queue.write();
    let len = queue.len();
    let mut i = 0;
    while i < len {
        let config = queue.pop_front().unwrap(); // This should not panic
        if let Some(audio_source) = sources.get(&config.source_handle) {
            let (handle, split) = oddio::split(audio_source.to_signal(config.settings));
            let sink = audio_output.play::<Source>(split);
            // Unlike bevy_audio, we should not drop this
            let sink_handle = sink_assets.set(config.stop_handle, sink);
            sinks.insert(sink_handle.id, sink_handle.clone());

            let audio_handle = AudioHandle(ManuallyDrop::new(handle));
            let signal_handle = handle_assets.set(config.audio_handle, audio_handle);
            handles.insert(signal_handle.id, signal_handle.clone());
        } else {
            queue.push_back(config);
        }
        i += 1;
    }
}

/// Asset that controls the playback of the sound.
#[derive(TypeUuid, Deref, DerefMut)]
#[uuid = "82317ee9-8f2d-4973-bb7f-8f4a5b74cc55"]
pub struct AudioSink<Source: ToSignal + Asset>(
    ManuallyDrop<OddioHandle<Stop<SplitSignal<<Source as ToSignal>::Signal>>>>,
);

/// Storage of all audio sinks.
#[derive(Deref, DerefMut)]
pub struct AudioSinks<Source: ToSignal + Asset>(HashMap<HandleId, BevyHandle<AudioSink<Source>>>);

/// [`oddio::Handle`] asset for a given signal.  
#[derive(TypeUuid, Deref, DerefMut)]
#[uuid = "18b98538-c486-4355-8712-cbfc558a4994"]
pub struct AudioHandle<Source: ToSignal + Asset>(
    ManuallyDrop<OddioHandle<<Source as ToSignal>::Signal>>,
);

/// Storage of all audio handles.
#[derive(Deref, DerefMut)]
pub struct AudioHandles<Source: ToSignal + Asset>(
    HashMap<HandleId, BevyHandle<AudioHandle<Source>>>,
);

impl<Source: ToSignal + Asset> Default for AudioSinks<Source> {
    fn default() -> Self {
        Self(HashMap::default())
    }
}

impl<Source: ToSignal + Asset> Default for AudioHandles<Source> {
    fn default() -> Self {
        Self(HashMap::default())
    }
}