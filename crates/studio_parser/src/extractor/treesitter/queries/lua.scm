; Functions: `function f()`, `function M.f()`, `function M:f()`, `local function f()`
(function_declaration name: (identifier) @func.name parameters: (parameters) @func.params) @func.def
(function_declaration
  name: (dot_index_expression table: (_) @func.receiver field: (identifier) @func.name)
  parameters: (parameters) @func.params) @func.def
(function_declaration
  name: (method_index_expression table: (_) @func.receiver method: (identifier) @func.name)
  parameters: (parameters) @func.params) @func.def
(assignment_statement
  (variable_list name: (identifier) @func.name)
  (expression_list value: (function_definition parameters: (parameters) @func.params))) @func.def

; Imports
((function_call
  name: (identifier) @_req
  arguments: (arguments (string content: (string_content) @import.path))) @import.def
  (#eq? @_req "require"))

; Calls
(function_call name: (identifier) @call.name)
(function_call name: (dot_index_expression field: (identifier) @call.name))
(function_call name: (method_index_expression method: (identifier) @call.name))
