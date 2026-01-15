TABOR-MSG(1)

# NAME

tabor-msg - Send messages to Tabor.

# SYNOPSIS

This command communicates with running Tabor instances through a socket,
making it possible to control Tabor without directly accessing it.

# OPTIONS

*-s, --socket* _<SOCKET>_

	Path for IPC socket communication.

# MESSAGES

*config*

	Update the Tabor configuration.

	*ARGS*
		*<CONFIG_OPTIONS>...*

			Configuration file options.

			Example: _tabor msg config 'cursor.style="Beam"'_

	*FLAGS*
		*-r, --reset*

			Clear all runtime configuration changes.

	*OPTIONS*
		*-w, --window-id* _<WINDOW_ID>_

			Window ID for the new config.

			Use _-1_ to apply this change to all windows.

			Default: _$TABOR_WINDOW_ID_

*get-config*

	Read runtime Tabor configuration.

	*OPTIONS*
		*-w, --window-id* _<WINDOW_ID>_

			Window ID for the config request.

			Use _-1_ to get the global config.

			Default: _$TABOR_WINDOW_ID_

Typed IPC commands map directly to IPC requests. Run
_tabor msg <command> --help_ for flags and arguments.

*ping*

	Health check (pong).

*get-capabilities*

	List IPC capabilities.

*list-tabs*

	List tabs grouped by tab group.

*get-tab-state*

	Get state for a single tab.

*create-tab*

	Create a new terminal tab or a web tab.
	Supports _--group-id_ or _--group-name_ to target a group.

*create-group*

	Create a new tab group.

*close-tab*

	Close a tab (defaults to active).

*select-tab*

	Select a tab by position or id.

*move-tab*

	Move a tab within or across groups.

*set-tab-title*

	Set or clear a tab title.

*set-group-name*

	Set or clear a tab group name.

*restore-closed-tab*

	Restore the most recently closed tab.

*open-url*

	Open a URL in the current or a new tab.

*set-web-url*

	Set the URL for a web tab.

*reload-web*

	Reload a web tab.

*open-inspector*

	Open Web Inspector for a web tab.

*get-tab-panel*

	Get tab panel state.

*set-tab-panel*

	Set tab panel state.

*dispatch-action*

	Dispatch a configured action.

*send-input*

	Send literal input text to a tab.

*run-command-bar*

	Open the command bar with input.

*inspector*

	Web Inspector subcommands: _list-targets_, _attach_, _detach_, _send_, _poll_.

*send*

	Send a raw JSON IPC message.

	*ARGS*
		*<JSON>*

			JSON payload to send to the IPC socket.

			Example: _tabor msg send '{"type":"list_tabs"}'_

			Inspector example:
			_tabor msg send '{"type":"send_inspector_message","session_id":"PID:12345-1","message":"{\"id\":1,\"method\":\"Network.enable\"}"}'_

	See _docs/ipc.md_ for the full IPC protocol and request list.

*list-requests*

	List available IPC request types with brief summaries.

	See _docs/ipc.md_ for the full IPC protocol and request schemas.

# SEE ALSO

*tabor*(1), *tabor*(5), *tabor-bindings*(5)

# BUGS

Found a bug? Please report it at _https://github.com/tartavull/tabor/issues_.

# MAINTAINERS

- Christian Duerr <contact@christianduerr.com>
- Kirill Chibisov <contact@kchibisov.com>
