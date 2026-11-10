(function () {
  "use strict";

  const SDK = window.__HERMES_PLUGIN_SDK__;
  if (!SDK) return;

  const React = SDK.React;
  const hooks = SDK.hooks;
  const { useState, useEffect, useRef } = hooks;
  const { Card, CardHeader, CardTitle, CardContent } = SDK.components;

  const BASE_URL = "http://localhost:8386/api/v1/video/frame";

  function DosboxVideoTab() {
    const [mode, setMode] = useState("rendered");
    const [quality, setQuality] = useState(85);
    const [fps, setFps] = useState(15);
    const [frameUrl, setFrameUrl] = useState("");
    const [status, setStatus] = useState("waiting");
    const frameCount = useRef(0);
    const intervalRef = useRef(null);

    const buildUrl = () => {
      const params = new URLSearchParams({
        format: "jpeg",
        mode,
        quality: String(quality),
        t: String(Date.now()),
      });
      return `${BASE_URL}?${params.toString()}`;
    };

    const tick = () => {
      const url = buildUrl();
      setFrameUrl(url);
      frameCount.current += 1;
    };

    useEffect(() => {
      tick();
      intervalRef.current = setInterval(tick, Math.max(33, Math.floor(1000 / fps)));
      return () => {
        if (intervalRef.current) clearInterval(intervalRef.current);
      };
    }, [fps, mode, quality]);

    useEffect(() => {
      if (!frameUrl) return;
      const img = new Image();
      img.onload = () => setStatus("live");
      img.onerror = () => {
        const is503 = img.naturalWidth === 0 && img.naturalHeight === 0;
        setStatus(is503 ? "no-frame" : "error");
      };
      img.src = frameUrl;
    }, [frameUrl]);

    const statusLabel = {
      waiting: "Waiting for DOSBox webserver...",
      "no-frame": "No frame yet (503) — is webserver enabled and DOSBox running?",
      live: `Live — ${frameCount.current} frames`,
      error: "Error loading frame",
    }[status];

    const isLive = status === "live";

    const h = React.createElement;

    return h(Card, { className: "h-full flex flex-col" }, [
      h(CardHeader, { key: "header" },
        h(CardTitle, { className: "flex items-center gap-2" }, [
          "DOSBox Video",
          h("span", {
            key: "badge",
            className: "text-xs px-2 py-0.5 rounded " + (isLive ? "bg-green-900/50 text-green-400" : "bg-yellow-900/50 text-yellow-400")
          }, isLive ? "LIVE" : "WAITING"),
        ])
      ),
      h(CardContent, { key: "content", className: "flex-1 flex flex-col p-4 gap-4" }, [
        h("div", { key: "controls", className: "flex items-center gap-4 flex-wrap" }, [
          h("div", { key: "mode", className: "flex items-center gap-2" }, [
            h("label", { key: "label", className: "text-sm" }, "Mode"),
            h("select", {
              key: "select",
              value: mode,
              onChange: (e) => setMode(e.target.value),
              className: "w-40 px-2 py-1 border rounded bg-background text-foreground",
            }, [
              h("option", { key: "rendered", value: "rendered" }, "Rendered (post-shader)"),
              h("option", { key: "raw", value: "raw" }, "Raw (framebuffer)"),
            ]),
          ]),
          h("div", { key: "quality", className: "flex items-center gap-2" }, [
            h("label", { key: "label", className: "text-sm w-24" }, `Quality: ${quality}`),
            h("input", {
              key: "input",
              type: "range",
              min: 10,
              max: 100,
              step: 5,
              value: quality,
              onChange: (e) => setQuality(Number(e.target.value)),
              className: "w-48",
            }),
          ]),
          h("div", { key: "fps", className: "flex items-center gap-2" }, [
            h("label", { key: "label", className: "text-sm w-16" }, `FPS: ${fps}`),
            h("input", {
              key: "input",
              type: "range",
              min: 1,
              max: 30,
              step: 1,
              value: fps,
              onChange: (e) => setFps(Number(e.target.value)),
              className: "w-48",
            }),
          ]),
        ]),
        h("div", { key: "viewport", className: "flex-1 flex items-center justify-center relative min-h-[300px] bg-black/20 rounded" }, [
          frameUrl && h("img", {
            key: "frame",
            src: frameUrl,
            alt: "DOSBox frame",
            style: {
              maxWidth: "100%",
              maxHeight: "100%",
              objectFit: "contain",
              imageRendering: "pixelated",
            },
            className: isLive ? "opacity-100" : "opacity-50",
          }),
          !isLive && h("div", {
            key: "overlay",
            className: "absolute inset-0 flex items-center justify-center text-muted-foreground",
          }, statusLabel),
        ]),
        h("p", { key: "status", className: "text-xs text-muted-foreground text-center" }, statusLabel),
      ]),
    ]);
  }

  window.__HERMES_PLUGINS__.register("dosbox-full-auto", DosboxVideoTab);
})();