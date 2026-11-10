// Equivalent of `--disable-features=MediaSessionService`: the enhanced media service owns the
// system media session, so the page's Media Session API is replaced by an inert stub.
(function () {
  if (window.top !== window || !("mediaSession" in navigator)) return;
  const stub = {
    metadata: null,
    playbackState: "none",
    setActionHandler() {},
    setPositionState() {},
    setMicrophoneActive() {},
    setCameraActive() {}
  };
  try {
    Object.defineProperty(Navigator.prototype, "mediaSession", { get: () => stub, configurable: true });
  } catch (err) {
    console.warn("[ytmd] could not disable media session", err);
  }
})();
