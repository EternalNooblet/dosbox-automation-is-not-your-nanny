// This file is part of the dosbox-automation Project.
// License: GPL-2.0-or-later. Contact: dosbox-automation-project@trinity2k.net
//

// Changed from upstream dosbox-automation in this fork (this repo is a fork, not the original upstream project).
// Last change to this file: 2026-11-01. Fork base commit: b9f24198a. Per-file history: git log -- <this file> in this repo.

#ifndef DOSBOX_WEBSERVER_BRIDGE_H
#define DOSBOX_WEBSERVER_BRIDGE_H

#include <chrono>
#include <condition_variable>
#include <mutex>
#include <vector>

namespace Webserver {

class Command {
public:
	virtual ~Command() {}
	virtual void Execute() = 0;

	void WaitForCompletion(const uint32_t timeout_ms = 1000);

	// Set by Execute() to report errors without throwing on the
	// emulation thread. Handlers should check this after
	// WaitForCompletion() and throw on the webserver thread.
	std::string error = {};

private:
	friend class Bridge;

	bool done = false;
};

class Bridge {
public:
	static Bridge& Instance();

	// Called by the web server thread
	void ExecuteCommand(Command& cmd, const uint32_t timeout_ms);

	// Called by the main thread running the CPU emulation
	void ProcessRequests();

private:
	std::mutex mtx                   = {};
	std::condition_variable cv       = {};
	std::vector<Command*> queue      = {};

	Bridge(const Bridge&)            = delete;
	Bridge& operator=(const Bridge&) = delete;
	Bridge()                         = default;
};

} // namespace Webserver

#endif // DOSBOX_WEBSERVER_BRIDGE_H
