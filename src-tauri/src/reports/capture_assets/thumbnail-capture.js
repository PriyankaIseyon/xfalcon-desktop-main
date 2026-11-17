(function () {
  "use strict";
  var renderer = window.htmlToImage;
  var nonce = document.currentScript.dataset.captureNonce;
  // Both documents may have opaque origins. Accept a private reply channel
  // from our direct parent instead of sending dashboard pixels to a window
  // whose origin (or document after navigation) we cannot authenticate.
  var connected = new Promise(function (resolve) {
    function connect(event) {
      var msg = event.data;
      if (!event.isTrusted || event.source !== window.parent || !msg ||
          msg.type !== "ida-thumbnail-connect" || msg.nonce !== nonce ||
          event.ports.length !== 1) { return; }
      event.stopImmediatePropagation();
      window.removeEventListener("message", connect, true);
      var port = event.ports[0];
      resolve(function (value) {
        port.postMessage(Object.assign({type: "ida-thumbnail", nonce: nonce}, value));
        port.close();
      });
    }
    // Installed before author scripts; keep the port out of their listeners.
    window.addEventListener("message", connect, true);
  });
  function delay(ms) { return new Promise(function (resolve) { setTimeout(resolve, ms); }); }
  async function capture() {
    var send = await connected;
    try {
      if (document.fonts) { await document.fonts.ready; }
      await Promise.all(Array.from(document.images).map(function (img) {
        return img.complete ? Promise.resolve() : new Promise(function (resolve) {
          img.addEventListener("load", resolve, {once: true});
          img.addEventListener("error", resolve, {once: true});
        });
      }));
      await new Promise(function (resolve) {
        var timer = setTimeout(resolve, 250);
        requestAnimationFrame(function () { requestAnimationFrame(function () {
          clearTimeout(timer); resolve();
        }); });
      });
      await delay(1200);
      var width = Math.min(window.innerWidth, 2048);
      // Include charts below the viewer's first fold without changing its layout.
      // Bound the source surface as well as the encoded image dimensions.
      var height = Math.min(Math.max(window.innerHeight,
        document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0), 2048);
      var targetWidth = Math.min(960, width, Math.floor(2048 * width / height));
      var options = {
        width: width, height: height, canvasWidth: targetWidth,
        canvasHeight: Math.max(1, Math.round(targetWidth * height / width)),
        pixelRatio: 1, skipFonts: true, backgroundColor: "#ffffff"
      };
      // WebKit's first foreignObject paint can omit decoded canvas images.
      // Warm its image cache once, then retain the second render. Both passes
      // stay inside the existing capture deadline and private reply channel.
      await renderer.toPng(document.documentElement, options);
      await delay(100);
      // Dense pages can exceed the storage limit at 960px. Preserve the whole
      // bounded page and reduce resolution instead of failing or cropping it.
      for (var attempt = 0; attempt < 4; attempt++) {
        var png = await renderer.toPng(document.documentElement, options);
        var body = png.slice(png.indexOf(",") + 1);
        var bytes = body.length * 3 / 4 - (body.endsWith("==") ? 2 : body.endsWith("=") ? 1 : 0);
        if (bytes <= 204800) { send({image_base64: body}); return; }
        targetWidth = Math.floor(targetWidth * 0.8);
        options.canvasWidth = targetWidth;
        options.canvasHeight = Math.max(1, Math.round(targetWidth * height / width));
      }
      throw new Error("Thumbnail exceeds image limit");
    } catch (_) { send({error: "Thumbnail capture failed"}); }
  }
  window.addEventListener("load", capture, {once: true});
})();
