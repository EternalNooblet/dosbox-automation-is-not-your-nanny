// This file is part of the dosbox-automation Project.
// License: GPL-2.0-or-later. Contact: dosbox-automation-project@trinity2k.net
//

// Changed from upstream dosbox-automation in this fork (this repo is a fork, not the original upstream project).
// Last change to this file: 2026-11-04. Fork base commit: b9f24198a. Per-file history: git log -- <this file> in this repo.

#include "webserver.h"
#include "bridge.h"
#include "capture.h"
#include "control.h"
#include "drive.h"
#include "input.h"
#include "mixer.h"
#include "private/auth.h"
#include "private/cpu.h"
#include "private/dos.h"
#include "private/freeze.h"
#include "private/io_port.h"
#include "private/memory.h"
#include "private/shutdown.h"
#include "private/tools.h"
#include "video.h"

#include "lua/lua_bridge_commands.h"

#include "dos/programs/mount_policy.h"
#include "gui/mapper.h"
#include "gui/osd/osd.h"
#include "gui/osd/osd_port.h"

#include <atomic>
#include <cctype>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <mutex>
#include <set>
#include <stdexcept>
#include <string>
#include <thread>

#if defined(WIN32)
#include <bcrypt.h>
#include <windows.h>
#endif

#include "http/http.h"
#include "nlohmann/json.hpp"

#include "augra/log.h"
#include "config/config.h"
#include "dosbox.h"
#include "misc/cross.h"
#include "misc/logging.h"
#include "misc/support.h"

using json = nlohmann::json;

// Set at init, cleared by the webserver thread when the bind fails.
static void clear_endpoint();

namespace Webserver {

void send_json(httplib::Response& res, const nlohmann::json& j)
{
	res.set_content(j.dump(2, ' ', false, nlohmann::json::error_handler_t::replace),
	                "application/json");
}

static void error_handler(const httplib::Request&, httplib::Response& res,
                          std::exception_ptr ep)
{
	json j;
	std::string msg;
	res.status = httplib::StatusCode::InternalServerError_500;

	try {
		if (ep) {
			std::rethrow_exception(ep);
		}
	} catch (const std::invalid_argument&) {
		msg        = "Invalid request parameter";
		res.status = httplib::StatusCode::BadRequest_400;
	} catch (const std::out_of_range&) {
		msg        = "Address out of range";
		res.status = httplib::StatusCode::BadRequest_400;
	} catch (const nlohmann::json::exception&) {
		msg        = "Malformed request body";
		res.status = httplib::StatusCode::BadRequest_400;
	} catch (const std::exception&) {
		msg = "Internal server error";
	} catch (...) {
		msg = "Internal server error";
	}

	j["error"] = msg;
	send_json(res, j);
}

bool IsPublicDocPath(const std::string& method, const std::string& path)
{
	if (method != "GET" && method != "HEAD") {
		return false;
	}

	// Exact match only. A prefix or normalized match would let an
	// encoded traversal escape the static mounts.
	static const std::set<std::string> public_paths = {
	        "/",
	        "/index.html",
	        "/style-index.css",
	        "/api.html",
	        "/openapi.json",
	        "/swagger-ui.css",
	        "/swagger-ui-bundle.js",
	        "/cheat-workbench.html",
	        "/tools/cheat-workbench.html",
	        "/favicon.ico",
	};
	return public_paths.count(path) > 0;
}

static httplib::Server server;

static void setup_api_handlers()
{
	server.Get("/api/v1/cpu/state", CpuStateCommand::Get);
	server.Get("/api/v1/cpu/cycles", CyclesCommand::Get);
	server.Put("/api/v1/cpu/cycles", CyclesCommand::Put);

	server.Get("/api/v1/dos/internals", DosInternalsCommand::Get);

	server.Post("/api/v1/dosbox/shutdown", ShutdownCommand::Post);

	server.Post("/api/v1/memory/allocate", AllocMemoryCommand::Post);
	server.Post("/api/v1/memory/free", FreeMemoryCommand::Post);
	server.Post("/api/v1/memory/search", SearchMemoryCommand::Post);
	server.Post("/api/v1/memory/freeze", FreezeHandlers::Post);
	server.Get("/api/v1/memory/freeze", FreezeHandlers::Get);
	server.Delete("/api/v1/memory/freeze", FreezeHandlers::Delete);
	server.Get("/api/v1/memory/pagemap", PageMapCommand::Get);
	server.Get("/api/v1/memory/:offset/:len", ReadMemoryCommand::Get);
	server.Get("/api/v1/memory/:segment/:offset/:len", ReadMemoryCommand::Get);
	server.Put("/api/v1/memory/:offset", WriteMemoryCommand::Put);
	server.Put("/api/v1/memory/:segment/:offset", WriteMemoryCommand::Put);

	server.Get("/api/v1/io/port", PortReadCommand::Get);
	server.Put("/api/v1/io/port", PortWriteCommand::Put);
	server.Put("/api/v1/cpu/register", WriteRegisterCommand::Put);

	server.Post("/api/v1/input/sequence", InputSequenceCommand::Post);
	server.Post("/api/v1/input/type", InputTypeCommand::Post);
	server.Get("/api/v1/input/mouse", MousePositionHandlers::Get);

	server.Get("/api/v1/video/frame", VideoHandlers::GetFrame);
	server.Get("/api/v1/video/frame/info", VideoHandlers::GetFrameInfo);
	server.Get("/api/v1/video/text", ScreenTextCommand::Get);

	server.Get("/api/v1/program/state", ControlHandlers::GetProgramState);
	server.Get("/api/v1/status", ControlHandlers::GetStatus);
	server.Post("/api/v1/control/shutdown", ShutdownCommand::Post);
	server.Post("/api/v1/control/pause", ControlHandlers::PostPause);
	server.Post("/api/v1/control/resume", ControlHandlers::PostResume);
	server.Get("/api/v1/control/paused", ControlHandlers::GetPaused);

	server.Post("/api/v1/drive/swap", DriveSwapCommand::Post);

	server.Post("/api/v1/mount/lock",
			[](const httplib::Request& req, httplib::Response& res) {
			bool want_locked = true;
			if (!req.body.empty()) {
				try {
					auto body = json::parse(req.body);
					want_locked = body.value("locked", true);
				} catch (...) {
					res.status = httplib::StatusCode::BadRequest_400;
					json err;
					err["error"] = "body must be JSON with a 'locked' boolean";
					send_json(res, err);
					return;
				}
			}
			if (want_locked) {
				MountPolicy::Lock();
			} else {
				MountPolicy::Unlock();
			}
			json j;
			j["status"] = want_locked ? "locked" : "unlocked";
			j["locked"] = want_locked;
			send_json(res, j);
			});
		server.Get("/api/v1/mount/lock",
			[](const httplib::Request&, httplib::Response& res) {
			json j;
			j["locked"] = MountPolicy::IsLocked();
			send_json(res, j);
			});

		server.Post("/api/v1/input/record/start", RecordingHandlers::PostStart);
	server.Post("/api/v1/input/record/pause", RecordingHandlers::PostPause);
	server.Post("/api/v1/input/record/stop", RecordingHandlers::PostStop);
	server.Get("/api/v1/input/record/status", RecordingHandlers::GetStatus);

	server.Post("/api/v1/script/load", Lua::LuaLoadCommand::Post);
	server.Post("/api/v1/script/start", Lua::LuaStartCommand::Post);
	server.Post("/api/v1/script/stop", Lua::LuaStopCommand::Post);
	server.Get("/api/v1/script/status", Lua::LuaStatusCommand::Get);

	server.Post("/api/v1/capture/video/start", CaptureStartCommand::Post);
	server.Post("/api/v1/capture/video/stop", CaptureStopCommand::Post);
	server.Get("/api/v1/capture/video/status", CaptureStatusCommand::Get);
	server.Get("/api/v1/capture/video/compression",
	           CaptureCompressionGetCommand::Get);
	server.Put("/api/v1/capture/video/compression",
	           CaptureCompressionSetCommand::Put);

	server.Post("/api/v1/capture/audio/start", AudioCaptureStartCommand::Post);
	server.Post("/api/v1/capture/audio/stop", AudioCaptureStopCommand::Post);
	server.Get("/api/v1/capture/audio/status", AudioCaptureStatusCommand::Get);

	server.Get("/api/v1/mixer", MixerStatusCommand::Get);
	server.Put("/api/v1/mixer/volume", MixerSetVolumeCommand::Put);
	server.Post("/api/v1/mixer/mute", MixerMuteCommand::Post);
	server.Post("/api/v1/mixer/unmute", MixerUnmuteCommand::Post);
	server.Put("/api/v1/mixer/channel/:name", MixerSetChannelVolumeCommand::Put);
}

static void setup_security(const std::string& addr, int port)
{
	(void)addr;
	(void)port;

	// Host-header validation removed: the server is bound to 0.0.0.0 on
	// purpose, so any Host is accepted and every request may reach the
	// API, with no Authorization header. Loopback tool pages still
	// announce themselves so a second workbench tab is not opened beside
	// an already-open one.
	server.set_pre_routing_handler(
	    [](const httplib::Request& req, httplib::Response& res) {
		if (IsLoopbackPeer(req.remote_addr)) {
			NoteToolPageSeen(req.get_header_value("X-DOSBox-Tool"),
			                 std::chrono::steady_clock::now());
		}

		return httplib::Server::HandlerResponse::Unhandled;
	});

	server.set_default_headers({
	        {"X-Content-Type-Options", "nosniff"},
	});

	server.Options(".*", [](const httplib::Request&, httplib::Response& res) {
		res.status = httplib::StatusCode::Forbidden_403;
	});

	server.set_payload_max_length(10 * 1024 * 1024);

	// httplib closes an idle keep-alive socket after 5 s, and a pooled
	// client reusing it in that instant gets no answer: close after every
	// response instead.
	server.set_keep_alive_max_count(1);
}

// Tool pages live outside the static mounts because httplib serves a
// mounted file before any registered handler runs. Loaded once: the
// page does not change while the engine runs. The page no longer
// receives an API token; its token field is unused.
static void setup_tool_pages()
{
	const auto page_path = get_resource_path("webserver-tools") /
	                       "cheat-workbench.html";
	std::string page;
	{
		auto in = std::ifstream(page_path, std::ios::binary);
		if (!in) {
			augra::log_warn("webserver",
			                "tool page missing: %s",
			                page_path.string().c_str());
		} else {
			page.assign(std::istreambuf_iterator<char>(in), {});
		}
	}

	server.Get("/tools/cheat-workbench.html",
	           [page](const httplib::Request&, httplib::Response& res) {
	           if (page.empty()) {
	               res.status = httplib::StatusCode::NotFound_404;
	               res.set_content("tool page not installed",
	                               "text/plain");
	               return;
	           }
	           res.set_header("Cache-Control", "no-store");
	           res.set_content(page, "text/html; charset=utf-8");
	           });

	// The pre-0.85.1 address; bookmarks and the primer point at it.
	server.Get("/cheat-workbench.html",
	           [](const httplib::Request&, httplib::Response& res) {
	           res.set_redirect("/tools/cheat-workbench.html",
	                            httplib::StatusCode::MovedPermanently_301);
	           });
}

static void run(const std::string addr, const int port,
                const std::string resource_home)
{
	const auto config_home = (get_config_dir() / DefaultWebserverDir).string();

	server.set_mount_point("/", config_home);
	server.set_mount_point("/", resource_home);

	setup_api_handlers();
	setup_tool_pages();
	setup_security(addr, port);

	server.set_exception_handler(error_handler);

	server.Get("/api/v1/dosbox/info",
	           [](const httplib::Request&, httplib::Response& res) {
	           json j;
	           j["version"]  = DOSBOX_GetDetailedVersion();
	           j["features"] = {
	                   {       "memory",  true},
	                   {        "input",  true},
	                   {"cpu_registers",  true},
	                   {  "cpu_control",  true},
	                   {      "port_io",  true},
	                   {       "freeze",  true},
	                   {     "debugger", false},
	           };
	           // For AppImage runs the executable path points into
	           // the transient squashfs mount; $APPIMAGE is the
	           // relaunchable file. Launcher generators prefer
	           // appimage over binary.
	           j["binary"] = get_executable_file().string();
	           const char* appimage = std::getenv("APPIMAGE");
	           j["appimage"] = appimage ? json(appimage) : json(nullptr);
	           send_json(res, j);
	           });

	augra::log_info("webserver",
	                "starting HTTP REST API on http://%s:%d",
	                addr.c_str(),
	                port);

	// httplib's default adds SO_REUSEPORT, which lets a second engine bind
	// beside a running one and split its requests; Windows needs the
	// exclusive flag for the same refusal.
	server.set_socket_options([](socket_t sock) {
		int yes = 1;
#ifdef _WIN32
		setsockopt(sock,
		           SOL_SOCKET,
		           SO_EXCLUSIVEADDRUSE,
		           reinterpret_cast<const char*>(&yes),
		           sizeof(yes));
#else
		setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof(yes));
#endif
	});

	auto ok = server.listen(addr, port);
	if (!ok) {
		augra::log_warn("webserver",
		                "failed to bind to %s:%d, another instance is "
		                "probably using the port; the API is off",
		                addr.c_str(),
		                port);
		clear_endpoint();
	}
}

static void init_config_settings(SectionProp& section)
{
	using enum Property::Changeable::Value;

	auto enabled = section.AddBool("webserver_enabled", OnlyAtStart, false);
	enabled->SetHelp(
	        "Enable the HTTP REST API that exposes internal state and memory (disabled by\n"
	        "default). Open http://localhost:8386 in a browser (or use the configured port)\n"
	        "to view the API documentation.\n"
	        "\n"
	        "The API is unauthenticated and no token is generated. Any\n"
	        "process that can reach the port can drive the machine; the\n"
	        "Host header check still applies to every request.");

	auto bind_ip = section.AddString("webserver_bind_address",
	                                 OnlyAtStart,
	                                 "127.0.0.1");
	bind_ip->SetHelp(
	        "Bind to the given IP address. By default only local connections are allowed.");

	auto bind_port = section.AddInt("webserver_port", OnlyAtStart, 8386);
	bind_port->SetMinMax(1, 0xFFFF);
	bind_port->SetHelp("TCP port to bind to.");

	auto allow_remote = section.AddBool("webserver_allow_remote", OnlyAtStart, false);
	allow_remote->SetHelp(
	        "Allow binding to non-localhost addresses (0.0.0.0 or ::).");

	auto token_file = section.AddBool("webserver_token_file", OnlyAtStart, false);
	token_file->SetHelp(
	        "(Deprecated; ignored. The API no longer issues tokens, so no token file is written.)");

	auto tool_token = section.AddString("webserver_tool_token", OnlyAtStart, "embed");
	tool_token->SetValues({"embed", "prompt"});
	tool_token->SetHelp(
	        "(Deprecated; ignored. Tool pages no longer receive an API\n"
	        "token; the token field in the page is unused.)");

	auto osd = section.AddBool("webserver_osd", OnlyAtStart, true);
	osd->SetHelp(
	        "Show on-screen indicators while automation is driving the machine\n"
	        "(script running, recording, replay, injected input). Enabled by\n"
	        "default so it is always clear when the machine is under remote\n"
	        "control. Set to false to hide the overlay.");

	// Mount policy no longer enforces a whitelist. These settings are kept
	// as no-ops for config compatibility (no unknown-setting warnings).
	// They have no effect on mount behavior.
}

} // namespace Webserver

// Written on the main thread at init and cleared from the webserver
// thread when the bind fails, read from the emulation thread.
static std::atomic<bool> is_webserver_enabled = false;
static std::mutex endpoint_mutex;
static std::optional<WebserverEndpoint> endpoint = {};

static void clear_endpoint()
{
	const std::lock_guard lock(endpoint_mutex);
	endpoint.reset();
	is_webserver_enabled = false;
}

static bool is_remote_address(const std::string& addr)
{
	return addr == "0.0.0.0" || addr == "::";
}

void WEBSERVER_Init()
{
	MAPPER_AddHandler(WEBSERVER_WorkbenchHotkey,
	                  SDL_SCANCODE_W,
	                  PRIMARY_MOD | MMOD2,
	                  "workbench",
	                  "Workbench");

	MountPolicy::InitPolicyConfig(get_primary_config_path());

	auto section = get_section("webserver");

	if (section->GetBool("webserver_enabled")) {
		const auto addr = section->GetString("webserver_bind_address");

		if (is_remote_address(addr)) {
			augra::log_info("webserver",
			                "binding to %s - API is exposed to the network",
			                addr.c_str());
		}

		is_webserver_enabled = true;
		const auto port      = section->GetInt("webserver_port");
		{
			const std::lock_guard lock(endpoint_mutex);
			endpoint = WebserverEndpoint{addr, port};
		}

		// Runs before AUTOEXEC_Init, so autoexec MOUNT and BOOT are
		// covered by the whitelist from the first line onwards.
		MountPolicy::SetInjectionPossible(true);

		OSD::OsdManager::Instance().SetEnabled(
		        section->GetBool("webserver_osd"));

		// Guest-side OSD access (osd.com) is part of the
		// automation surface, so it comes and goes with it
		OSDPORT_EnableOverlay(true);

		const auto resource_home = get_resource_path("webserver").string();
		Webserver::InputRecording::InstallHooks();

		std::thread thread(Webserver::run,
		                   addr,
		                   port,
		                   resource_home);

		thread.detach();
	}
}

void WEBSERVER_Destroy()
{
	OSDPORT_EnableOverlay(false);
	Webserver::server.stop();
}

void WEBSERVER_AddConfigSection(const ConfigPtr& conf)
{
	assert(conf);

	auto section = conf->AddSection("webserver");

	Webserver::init_config_settings(*section);
}

bool WEBSERVER_IsEnabled()
{
	return is_webserver_enabled;
}

// Written by the webserver thread, read from the emulation thread
// on every pass of the machine loop.
static std::atomic<bool> machine_paused = false;

void WEBSERVER_SetPaused(const bool paused)
{
	machine_paused = paused;
}

bool WEBSERVER_IsPaused()
{
	return machine_paused.load();
}

std::optional<WebserverEndpoint> WEBSERVER_GetEndpoint()
{
	const std::lock_guard lock(endpoint_mutex);
	return endpoint;
}
