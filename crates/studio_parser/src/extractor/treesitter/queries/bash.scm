(function_definition name: (word) @func.name) @func.def

((command
  name: (command_name (word) @_src)
  argument: (_) @import.path) @import.def
  (#any-of? @_src "source" "."))

(command name: (command_name (word) @call.name))
