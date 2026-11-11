// This file is part of the dosbox-automation Project.
// License: GPL-2.0-or-later. Contact: dosbox-automation-project@trinity2k.net
//

// Changed from upstream dosbox-automation in this fork (this repo is a fork, not the original upstream project).
// Last change to this file: 2026-11-02. Fork base commit: b9f24198a. Per-file history: git log -- <this file> in this repo.

#include "bridge.h"

#include <algorithm>
#include <chrono>
#include <mutex>
#include <vector>

namespace Webserver {

Bridge& Bridge::Instance()
{
	static Bridge instance;
	return instance;
}

void Command::WaitForCompletion(const uint32_t timeout_ms)
{
	Bridge::Instance().ExecuteCommand(*this, timeout_ms);
}

void Bridge::ExecuteCommand(Command& cmd, const uint32_t timeout_ms)
{
	// Bounded lock acquisition: if the emulation thread is wedged inside
	// a command's Execute() it holds this mutex forever. An unbounded
	// lock would then hang every API thread until the HTTP client's 30 s
	// timeout (observed 2026-11-02: one wedged memory command made every
	// later request unanswered). Fail fast with a distinct message so a
	// wedge is visible instead of turning into silent timeouts.
	std::unique_lock<std::mutex> lock(mtx, std::defer_lock);
	const auto deadline = std::chrono::steady_clock::now() +
	                      std::chrono::milliseconds(timeout_ms);
	while (!lock.try_lock()) {
		if (std::chrono::steady_clock::now() >= deadline) {
			throw std::runtime_error("Command bridge stalled (emulator thread holds the queue)");
		}
		std::this_thread::sleep_for(std::chrono::milliseconds(5));
	}

	cmd.done = false;
	queue.push_back(&cmd);

	bool success = cv.wait_for(lock,
	                           std::chrono::milliseconds(timeout_ms),
	                           [&] { return cmd.done; });

	if (!success) {
		auto it = std::find(queue.begin(), queue.end(), &cmd);
		if (it != queue.end()) {
			queue.erase(it);
		}
		throw std::runtime_error("Failed to execute command: timeout");
	}
}

void Bridge::ProcessRequests()
{
	std::lock_guard<std::mutex> lock(mtx);

	if (queue.empty()) {
		return;
	}
	for (auto* cmd : queue) {
		cmd->Execute();
		cmd->done = true;
	}

	queue.clear();
	cv.notify_all();
}

} // namespace Webserver
