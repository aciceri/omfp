//! Video and audio files in `omfp:file` (E2E-51), against a daemon on the fake launcher: a WAV tone made by the test
//! and a `video/webm` clip the browser records from a canvas load in the browser's own players from their raw bytes
//! (DMN-250), the tone seeks, and the shots show both (`omfp-media-audio.png`, `omfp-media-video.png`).

use e2e::Journey;
use omfp_daemon::agent::FakeLauncher;
use serde_json::{Value, json};

const AUDIO: &str = "[data-testid=file-media][data-kind=audio] audio[data-testid=file-player]";
const VIDEO: &str = "[data-testid=file-media][data-kind=video] video[data-testid=file-player]";

/// A WAV file: `seconds` of a 440 Hz tone, 8 kHz mono 16-bit PCM.
fn wav(seconds: u32) -> Vec<u8> {
    const RATE: u32 = 8000;
    let samples = RATE * seconds;
    let data = samples * 2;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in 0..samples {
        let phase = f64::from(sample) * 440.0 * std::f64::consts::TAU / f64::from(RATE);
        // Within ±8000, so the cast cannot truncate.
        #[allow(clippy::cast_possible_truncation)]
        let value = (phase.sin() * 8000.0) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// A few frames of a 64 × 48 canvas recorded by the browser as `video/webm` (no encoder in the test).
async fn recorded_webm(journey: &Journey) -> anyhow::Result<Vec<u8>> {
    let value = journey
        .eval(
            "return (async () => {
               const canvas = document.createElement('canvas');
               canvas.width = 64;
               canvas.height = 48;
               const context = canvas.getContext('2d');
               const recorder = new MediaRecorder(canvas.captureStream(30), { mimeType: 'video/webm' });
               const chunks = [];
               recorder.ondataavailable = (event) => chunks.push(event.data);
               const stopped = new Promise((resolve) => { recorder.onstop = resolve; });
               recorder.start();
               for (let frame = 0; frame < 12; frame++) {
                 context.fillStyle = `hsl(${frame * 30}, 80%, 50%)`;
                 context.fillRect(0, 0, 64, 48);
                 await new Promise((resolve) => requestAnimationFrame(resolve));
               }
               recorder.stop();
               await stopped;
               return Array.from(new Uint8Array(await new Blob(chunks).arrayBuffer()));
             })();",
            Vec::new(),
        )
        .await?;
    let bytes = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no recording: {value}"))?
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(|| anyhow::anyhow!("not bytes"))?;
    anyhow::ensure!(bytes.starts_with(b"\x1a\x45\xdf\xa3"), "not WebM");
    Ok(bytes)
}

/// `[readyState, duration, currentTime, videoWidth, error code]` of the first element matching `css`.
async fn media_state(journey: &Journey, css: &str) -> anyhow::Result<Value> {
    journey
        .eval(
            "const m = document.querySelector(arguments[0]);
             return m ? [m.readyState, Number.isFinite(m.duration) ? m.duration : -1, m.currentTime,
                         m.videoWidth ?? 0, m.error ? m.error.code : 0] : null;",
            vec![json!(css)],
        )
        .await
}

/// Waits until the player `css` shows and has its metadata (`readyState` ≥ 1); returns its state.
async fn until_loaded(journey: &Journey, css: &str) -> anyhow::Result<Value> {
    journey
        .until(&format!("{css} to load"), async |page| {
            let state = media_state(page, css).await?;
            if state.is_null() {
                return Ok(None);
            }
            anyhow::ensure!(state[4] == json!(0), "the player failed: {state}");
            Ok((state[0].as_u64().unwrap_or_default() >= 1).then_some(state))
        })
        .await
}

// spec: E2E-51, UI-541, DMN-250, DMN-252
#[tokio::test(flavor = "multi_thread")]
async fn video_and_audio_files_play_from_their_raw_bytes() -> anyhow::Result<()> {
    let journey = Journey::start_fake(FakeLauncher::new()).await?;
    journey.connected().await?;
    let thread = journey.create_thread("media").await?;
    let folder = journey.workspace_folder(&thread).await?;
    std::fs::create_dir_all(folder.join("media"))?;
    std::fs::write(folder.join("media/tone.wav"), wav(2))?;
    std::fs::write(
        folder.join("media/clip.webm"),
        recorded_webm(&journey).await?,
    )?;

    journey.command("open media/tone.wav").await?;
    journey.element(AUDIO).await?;
    let src = journey.attr(AUDIO, "src").await?.unwrap_or_default();
    assert!(
        src.starts_with("/files/") && src.contains("/media/tone.wav?v="),
        "the raw file URL: {src}"
    );
    let loaded = until_loaded(&journey, AUDIO).await?;
    let duration = loaded[1].as_f64().unwrap_or_default();
    assert!((duration - 2.0).abs() < 0.05, "two seconds: {loaded}");
    // Seeking reads from the middle of the file.
    journey
        .eval(
            "const m = document.querySelector(arguments[0]);
             return new Promise((resolve) => { m.onseeked = () => resolve(m.currentTime); m.currentTime = 1.5; });",
            vec![json!(AUDIO)],
        )
        .await?;
    let moved = media_state(&journey, AUDIO).await?;
    let time = moved[2].as_f64().unwrap_or_default();
    assert!((time - 1.5).abs() < 0.05, "at 1.5 s: {moved}");
    journey.screenshot("media-audio").await?;

    journey.command("open media/clip.webm").await?;
    let video = until_loaded(&journey, VIDEO).await?;
    assert_eq!(video[3], json!(64), "the frames' width: {video}");
    journey
        .fits("[data-testid=file-media][data-kind=video]")
        .await?;
    journey.screenshot("media-video").await?;
    journey.finish("E2E-51").await
}

// spec: UI-116, UI-117, UI-541, DMN-092
#[tokio::test(flavor = "multi_thread")]
async fn explicit_media_names_resolve_unique_ambiguous_and_missing_files() -> anyhow::Result<()> {
    let journey = Journey::start_fake(FakeLauncher::new()).await?;
    journey.connected().await?;
    let thread = journey.create_thread("explicit media names").await?;
    let folder = journey.workspace_folder(&thread).await?;
    std::fs::create_dir_all(folder.join("media"))?;
    std::fs::write(folder.join("media/tone.wav"), wav(1))?;

    // Both explicit forms reach the real player, regardless of prose path detection.
    for path in ["media/tone.wav", "tone.wav"] {
        journey.command(&format!("open {path}")).await?;
        let loaded = until_loaded(&journey, AUDIO).await?;
        assert_eq!(loaded[1].as_f64(), Some(1.0));
        journey
            .element("[data-testid=file][data-path=\"media/tone.wav\"]")
            .await?;
        journey
            .click(
                "[data-testid=file-tab][data-file=\"media/tone.wav\"] [data-testid=file-tab-close]",
            )
            .await?;
        journey.gone(AUDIO).await?;
    }

    std::fs::create_dir_all(folder.join("other"))?;
    std::fs::write(folder.join("other/tone.wav"), wav(2))?;
    journey.command("open tone.wav").await?;
    journey.until_focused("file-picker-input").await?;
    assert_eq!(
        journey
            .attrs("[data-testid=file-picker-option]", "data-id")
            .await?,
        [
            Some("media/tone.wav".to_owned()),
            Some("other/tone.wav".to_owned())
        ]
    );
    journey.keys(thirtyfour::prelude::Key::Down).await?;
    journey.keys(thirtyfour::prelude::Key::Enter).await?;
    journey
        .element("[data-testid=file][data-path=\"other/tone.wav\"]")
        .await?;
    assert_eq!(until_loaded(&journey, AUDIO).await?[1].as_f64(), Some(2.0));

    journey.command("open missing.wav").await?;
    journey
        .element("[data-testid=file][data-path=\"missing.wav\"] [data-testid=file-missing]")
        .await?;
    journey
        .screenshot("explicit-media-basename-missing")
        .await?;
    journey.finish("explicit-media-names").await
}
