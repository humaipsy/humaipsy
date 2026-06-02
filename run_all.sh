#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

URL_DASHBOARD="http://localhost:8080/"
URL_DOCS="http://localhost:8081/"
URL_HOST_RUNNER="http://localhost:8090/health"
ENABLE_DOCS=false

build_host_command() {
	printf '%s' "set -euo pipefail; trap 'rc=\$?; if [[ \$rc -ne 0 ]]; then echo; echo [run_all] host_runner failed with exit code \$rc; fi; exec bash' EXIT; cd \"$ROOT_DIR/platform\"; uv sync --dev; uv run uvicorn src.host_runner_main:app --host 0.0.0.0 --port 8090"
}

build_docker_command() {
	if [[ "$ENABLE_DOCS" == "true" ]]; then
		printf '%s' "set -euo pipefail; trap 'rc=\$?; if [[ \$rc -ne 0 ]]; then echo; echo [run_all] docker_compose failed with exit code \$rc; fi; exec bash' EXIT; cd \"$ROOT_DIR\"; docker compose up --build"
	else
		printf '%s' "set -euo pipefail; trap 'rc=\$?; if [[ \$rc -ne 0 ]]; then echo; echo [run_all] docker_compose failed with exit code \$rc; fi; exec bash' EXIT; cd \"$ROOT_DIR\"; docker compose up --build platform"
	fi
}

launch_terminal() {
	local terminal_cmd="$1"

	if command -v gnome-terminal >/dev/null 2>&1; then
		gnome-terminal -- bash -lc "$terminal_cmd" &
	elif command -v konsole >/dev/null 2>&1; then
		konsole -e bash -lc "$terminal_cmd" &
	elif command -v xfce4-terminal >/dev/null 2>&1; then
		xfce4-terminal --command "bash -lc \"$terminal_cmd\"" &
	elif command -v x-terminal-emulator >/dev/null 2>&1; then
		x-terminal-emulator -e bash -lc "$terminal_cmd" &
	elif command -v xterm >/dev/null 2>&1; then
		xterm -e bash -lc "$terminal_cmd" &
	elif command -v alacritty >/dev/null 2>&1; then
		alacritty -e bash -lc "$terminal_cmd" &
	else
		echo "No supported terminal emulator found. Install one of:"
		echo "gnome-terminal, konsole, xfce4-terminal, x-terminal-emulator, xterm, alacritty"
		return 1
	fi

	echo "$!"
}

open_urls() {
	if command -v google-chrome >/dev/null 2>&1; then
		if [[ "$ENABLE_DOCS" == "true" ]]; then
			google-chrome --new-window "$URL_DASHBOARD" --new-tab "$URL_DOCS" --new-tab "$URL_HOST_RUNNER" >/dev/null 2>&1 &
		else
			google-chrome --new-window "$URL_DASHBOARD" --new-tab "$URL_HOST_RUNNER" >/dev/null 2>&1 &
		fi
	elif command -v google-chrome-stable >/dev/null 2>&1; then
		if [[ "$ENABLE_DOCS" == "true" ]]; then
			google-chrome-stable --new-window "$URL_DASHBOARD" --new-tab "$URL_DOCS" --new-tab "$URL_HOST_RUNNER" >/dev/null 2>&1 &
		else
			google-chrome-stable --new-window "$URL_DASHBOARD" --new-tab "$URL_HOST_RUNNER" >/dev/null 2>&1 &
		fi
	elif command -v chromium >/dev/null 2>&1; then
		if [[ "$ENABLE_DOCS" == "true" ]]; then
			chromium --new-window "$URL_DASHBOARD" --new-tab "$URL_DOCS" --new-tab "$URL_HOST_RUNNER" >/dev/null 2>&1 &
		else
			chromium --new-window "$URL_DASHBOARD" --new-tab "$URL_HOST_RUNNER" >/dev/null 2>&1 &
		fi
	elif command -v chromium-browser >/dev/null 2>&1; then
		if [[ "$ENABLE_DOCS" == "true" ]]; then
			chromium-browser --new-window "$URL_DASHBOARD" --new-tab "$URL_DOCS" --new-tab "$URL_HOST_RUNNER" >/dev/null 2>&1 &
		else
			chromium-browser --new-window "$URL_DASHBOARD" --new-tab "$URL_HOST_RUNNER" >/dev/null 2>&1 &
		fi
	elif command -v xdg-open >/dev/null 2>&1; then
		xdg-open "$URL_DASHBOARD" >/dev/null 2>&1 &
		if [[ "$ENABLE_DOCS" == "true" ]]; then
			xdg-open "$URL_DOCS" >/dev/null 2>&1 &
		fi
		xdg-open "$URL_HOST_RUNNER" >/dev/null 2>&1 &
	else
		echo "Could not open browser automatically. Open these manually:"
		echo "  $URL_DASHBOARD"
		if [[ "$ENABLE_DOCS" == "true" ]]; then
			echo "  $URL_DOCS"
		fi
		echo "  $URL_HOST_RUNNER"
	fi
}

host_command="$(build_host_command)"
docker_command="$(build_docker_command)"

launch_terminal "$host_command" >/dev/null
launch_terminal "$docker_command" >/dev/null

open_urls

echo "Started services in new terminals."
echo "Dashboard:   $URL_DASHBOARD"
if [[ "$ENABLE_DOCS" == "true" ]]; then
	echo "Docs:        $URL_DOCS"
else
	echo "Docs:        disabled"
fi
echo "Host runner: $URL_HOST_RUNNER"
