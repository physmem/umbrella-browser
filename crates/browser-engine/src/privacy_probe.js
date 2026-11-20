// Injected into every document. Tells the browser when the page opens a
// WebRTC connection, which can reveal the device's real IP address even
// behind a VPN. The connection itself is left alone.
(() => {
  if (!window.chrome || !window.chrome.webview || !window.RTCPeerConnection) return;
  const Native = window.RTCPeerConnection;
  let told = false;
  function Watched(...args) {
    if (!told) {
      told = true;
      try { window.chrome.webview.postMessage({ umbrella: "webrtc" }); } catch (_) {}
    }
    return new Native(...args);
  }
  Watched.prototype = Native.prototype;
  Object.setPrototypeOf(Watched, Native);
  window.RTCPeerConnection = Watched;
  if (window.webkitRTCPeerConnection) window.webkitRTCPeerConnection = Watched;
})();
