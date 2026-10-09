; `const Foo = struct { ... }` / `enum { ... }`
(variable_declaration (identifier) @class.name (struct_declaration)) @class.def
(variable_declaration (identifier) @enum.name (enum_declaration)) @enum.def
(variable_declaration
  (identifier) @enum.name
  (enum_declaration (container_field name: (_) @enum.variant))) @enum.def
(struct_declaration (container_field name: (_) @field.name type: (_) @field.type) @field.def)

; Functions
(function_declaration name: (identifier) @func.name (parameters) @func.params) @func.def
(function_declaration name: (identifier) @func.name (parameters) @func.params type: (_) @func.return) @func.def

; Imports: const x = @import("path");
((variable_declaration
  (builtin_function
    (builtin_identifier) @_b
    (arguments (string (string_content) @import.path)))) @import.def
  (#eq? @_b "@import"))

; Calls
(call_expression function: (identifier) @call.name)
(call_expression function: (field_expression member: (identifier) @call.name))
