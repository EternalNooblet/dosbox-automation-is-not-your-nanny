// This file is part of the dosbox-automation-is-not-your-nanny fork
// License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.

(function () {
  "use strict";

  const SDK = window.__HERMES_PLUGIN_SDK__;
  if (!SDK) return;

  const React = SDK.React;
  const hooks = SDK.hooks;
  const { useState, useEffect, useRef } = hooks;
  const { Card, CardHeader, CardTitle, CardContent, Input, Button } = SDK.components;
  const { buildWsUrl } = SDK;

  // Use the dashboard's host for the webserver URL
  const WS_HOST = window.location.hostname || "localhost";
  const BASE_URL = `http://${WS_HOST}:8387/api/v1/video/frame`;

  function DosboxVideoTab() {
    const [mode, setMode] = useState("rendered");
    const [quality, setQuality] = useState(85);
    const [fps, setFps] = useState(15);
    const [frameUrl, setFrameUrl] = useState("");
    const [status, setStatus] = useState("waiting");
    const frameCount = useRef(0);
    const intervalRef = useRef(null);

    // Chat state
    const [messages, setMessages] = useState([]);
    const [chatInput, setChatInput] = useState("");
    const [sending, setSending] = useState(false);
    const wsRef = useRef(null);
    const requestIdRef = useRef(0);
    const sessionIdRef = useRef(null);
    const pendingRequestsRef = useRef(new Map());
    const scrollAreaRef = useRef(null);

    const scrollToBottom = () => {
      if (scrollAreaRef.current) {
        scrollAreaRef.current.scrollTop = scrollAreaRef.current.scrollHeight;
      }
    };
    const [wsStatus, setWsStatus] = useState("disconnected");

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
      waiting: "Waiting for dosbox-full-auto webserver...",
      "no-frame": "No frame yet (503) — is webserver enabled and dosbox-full-auto running?",
      error: "Error loading frame",
    }[status];

    const isLive = status === "live";

    // Connect to gateway WebSocket
    const connectWs = () => {
      if (wsRef.current?.readyState === WebSocket.OPEN) return Promise.resolve();
      return new Promise((resolve, reject) => {
        try {
          buildWsUrl("/api/ws").then(url => {
            console.log("[dosbox-full-auto] Connecting to WS:", url);
            const ws = new WebSocket(url);
            wsRef.current = ws;
            ws.onopen = () => {
              console.log("[dosbox-full-auto] WS connected");
              setWsStatus("connected");
              resolve();
            };
            ws.onclose = (e) => {
              console.log("[dosbox-full-auto] WS closed:", e.code, e.reason);
              setWsStatus("disconnected");
            };
            ws.onerror = (err) => {
              console.error("[dosbox-full-auto] WS error:", err);
              setWsStatus("error");
              reject(err);
            };
            ws.onmessage = (event) => {
              try {
                const msg = JSON.parse(event.data);
                handleWsMessage(msg);
              } catch (e) {
                console.log("[dosbox-full-auto] WS non-JSON:", event.data);
              }
            };
          }).catch(reject);
        } catch (err) {
          console.error("[dosbox-full-auto] WS connect failed:", err);
          reject(err);
        }
      });
    };

    const handleWsMessage = (msg) => {
      if (msg.id && msg.result && msg.result.session_id) {
        sessionIdRef.current = msg.result.session_id;
        console.log("[dosbox-full-auto] Session created:", msg.result.session_id);
      }
      if (msg.error) {
        console.error("[dosbox-full-auto] WS error response:", msg.error);
      }
      if (msg.method === "event") {
        handleEvent(msg.params);
      }
      if (msg.id && pendingRequestsRef.current.has(msg.id)) {
        const resolve = pendingRequestsRef.current.get(msg.id);
        pendingRequestsRef.current.delete(msg.id);
        resolve(msg);
      }
    };

    const handleEvent = (params) => {
          const { type, payload, session_id, seq } = params;
      
          if (type === "gateway.ready") {
            return;
          }
      
          if (type === "message.start") {
            setMessages(prev => [...prev, {
              role: "assistant",
              content: "",
              thinking: "",
              toolCalls: [],
              turnId: payload.turn_id,
              streaming: true
            }]);
            return;
          }
      
          if (type === "message.delta") {
            setMessages(prev => {
              const next = [...prev];
              const last = next[next.length - 1];
              if (last && last.role === "assistant" && last.streaming) {
                last.content = (last.content || "") + (payload.text || "");
              }
              return next;
            });
            return;
          }
      
          if (type === "thinking.delta") {
            setMessages(prev => {
              const next = [...prev];
              const last = next[next.length - 1];
              if (last && last.role === "assistant" && last.streaming) {
                last.thinking = (last.thinking || "") + (payload.text || "");
              }
              return next;
            });
            return;
          }
      
          // Skip tool events - don't render them in chat
          if (type === "tool.generating" || type === "tool.start" || type === "tool.complete") {
            return;
          }
      
          if (type === "message.complete") {
                  setMessages(prev => {
                    const next = [...prev];
                    const last = next[next.length - 1];
                    if (last && last.role === "assistant" && last.streaming) {
                      last.streaming = false;
                      last.content = payload.text || last.content;
                      last.usage = payload.usage;
                      last.reasoning = payload.reasoning;
                    }
                    return next;
                  });
                  setTimeout(scrollToBottom, 0);
                  return;
                }
        };

    const sendChat = async () => {
      if (!chatInput.trim() || sending) return;
      const message = chatInput;
      setChatInput("");
      setSending(true);

      try {
        await connectWs();
        const ws = wsRef.current;
        if (!ws || ws.readyState !== WebSocket.OPEN) {
          throw new Error("WebSocket not connected");
        }
        if (!sessionIdRef.current) {
          const createId = ++requestIdRef.current;
          ws.send(JSON.stringify({
            jsonrpc: "2.0",
            id: createId,
            method: "session.create",
            params: { profile: "default" }
          }));
          await new Promise((resolve) => {
            pendingRequestsRef.current.set(createId, resolve);
            setTimeout(resolve, 3000);
          });
        }
        if (!sessionIdRef.current) {
          throw new Error("No session available");
        }

        setMessages(prev => [...prev, {
          role: "user",
          content: message,
          timestamp: Date.now()
        }]);

        setTimeout(scrollToBottom, 0);

        const prefix = "load and use all dosbox-full-auto. use only the dosbox-full-auto-mcp commands to interact with dos apps. ";
        const enhancedMessage = prefix + message;

        const submitId = ++requestIdRef.current;
        ws.send(JSON.stringify({
          jsonrpc: "2.0",
          id: submitId,
          method: "prompt.submit",
          params: {
            session_id: sessionIdRef.current,
            text: enhancedMessage,
            surface: "dashboard-plugin"
          }
        }));
        console.log("[dosbox-full-auto] Sent prompt.submit:", message);
      } catch (err) {
        console.error("Chat send failed:", err);
        setChatInput(message);
      } finally {
        setSending(false);
      }
    };

    const h = React.createElement;

    return h(Card, { className: "h-full flex flex-col" }, [
      h(CardHeader, { key: "header" },
        h(CardTitle, { className: "flex items-center gap-2" }, "dosbox-full-auto")
      ),
      h(CardContent, { key: "content", className: "flex-1 flex flex-row p-4 gap-4 min-h-0" }, [
        // Video section (left)
        h("div", { key: "video", style: { flex: "1 1 0%", minWidth: 0 }, className: "flex flex-col gap-2" }, [
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
            h("span", { key: "ws-status", className: `text-xs px-2 py-1 rounded ${
              wsStatus === "connected" ? "text-green-400 bg-green-500/20" :
              wsStatus === "error" ? "text-red-400 bg-red-500/20" :
              "text-yellow-400 bg-yellow-500/20"
            }` }, wsStatus),
          ]),
          h("div", { key: "viewport", className: "flex-1 flex items-center justify-center relative bg-black/20 rounded" }, [
            frameUrl && h("img", {
              key: "frame",
              src: frameUrl,
              alt: "dosbox-full-auto frame",
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

        // Chat section (right)
        h("div", { key: "chat", style: { width: "20vw", maxWidth: "20vw", minWidth: "20vw", flexShrink: 0, flexGrow: 0, overflow: "hidden", maxHeight: "calc(100vh - 200px)" }, className: "flex flex-col gap-2 border-l pl-4 min-h-0" }, [
          h("div", { key: "scroll", ref: scrollAreaRef, className: "flex-1 min-h-0 pr-2 overflow-y-auto overflow-x-hidden" }, [
            messages.length === 0 ? h("div", {
              key: "empty",
              className: "flex items-center justify-center h-full text-muted-foreground text-sm"
            }, "Chat will appear here. Type below to start.") : messages.map((msg, i) => h("div", {
              key: i,
              className: `flex flex-col gap-1 ${msg.role === "user" ? "items-end" : "items-start"}`
            }, [
              msg.role === "user" && h("div", {
                key: "user",
                className: "bg-primary text-primary-foreground px-3 py-2 rounded-lg max-w-[80%] text-sm break-words overflow-wrap-anywhere"
              }, msg.content),
              msg.role === "assistant" && h("div", {
                key: "assistant",
                className: "w-full break-words"
              }, [
                msg.thinking && h("div", {
                  key: "thinking",
                  className: "text-xs text-muted-foreground italic bg-muted/50 px-2 py-1 rounded"
                }, msg.thinking),
                msg.content && h("div", {
                  key: "content",
                  className: "bg-muted px-3 py-2 rounded-lg max-w-[80%] text-sm break-words overflow-wrap-anywhere"
                }, msg.content),
                (msg.toolCalls && msg.toolCalls.length > 0) && h("div", {
                  key: "tools",
                  className: "mt-1 space-y-1 break-words"
                }, msg.toolCalls.map((tc, ti) => h("div", {
                  key: ti,
                  className: `text-xs px-2 py-1 rounded ${tc.status === "complete" ? "bg-green-500/20 text-green-400" : "bg-blue-500/20 text-blue-400"} break-words`
                }, `${tc.name}: ${JSON.stringify(tc.args || {})}${tc.result ? ` → ${JSON.stringify(tc.result).slice(0, 100)}` : ""}`))),
                msg.usage && h("div", {
                  key: "usage",
                  className: "text-xs text-muted-foreground mt-1"
                }, `Tokens: ${msg.usage.total} (in: ${msg.usage.input}, out: ${msg.usage.output})`),
              ]),
            ])),
          ]),
          h("div", { key: "input-row", className: "flex items-center gap-2" }, [
            h(Input, {
              key: "input",
              value: chatInput,
              onChange: (e) => setChatInput(e.target.value),
              onKeyDown: (e) => e.key === "Enter" && !e.shiftKey && (e.preventDefault(), sendChat()),
              placeholder: "Send to chat...",
              className: "flex-1",
              disabled: sending,
            }),
            h(Button, {
              key: "send",
              onClick: sendChat,
              disabled: sending || !chatInput.trim(),
              size: "sm",
            }, sending ? "Sending..." : "Send"),
          ]),
        ]),
      ]),
    ]);
  }

  window.__HERMES_PLUGINS__.register("dosbox-full-auto", DosboxVideoTab);
})();