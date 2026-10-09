; Structs (named, and typedef'd anonymous)
(struct_specifier name: (type_identifier) @class.name body: (field_declaration_list)) @class.def
(type_definition
  type: (struct_specifier body: (field_declaration_list))
  declarator: (type_identifier) @class.name) @class.def
(field_declaration type: (_) @field.type declarator: (field_identifier) @field.name) @field.def
(field_declaration type: (_) @field.type declarator: (pointer_declarator declarator: (field_identifier) @field.name)) @field.def
(field_declaration type: (_) @field.type declarator: (array_declarator declarator: (field_identifier) @field.name)) @field.def

; Enums
(enum_specifier name: (type_identifier) @enum.name body: (enumerator_list)) @enum.def
(enum_specifier
  name: (type_identifier) @enum.name
  body: (enumerator_list (enumerator name: (identifier) @enum.variant))) @enum.def
(type_definition
  type: (enum_specifier body: (enumerator_list (enumerator name: (identifier) @enum.variant)))
  declarator: (type_identifier) @enum.name) @enum.def

; Functions
(function_definition
  type: (_) @func.return
  declarator: (function_declarator
    declarator: (identifier) @func.name
    parameters: (parameter_list) @func.params)) @func.def
(function_definition
  type: (_) @func.return
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @func.name
      parameters: (parameter_list) @func.params))) @func.def

; Includes
(preproc_include path: (string_literal (string_content) @import.path)) @import.def
(preproc_include path: (system_lib_string) @import.path) @import.def

; Calls
(call_expression function: (identifier) @call.name)
(call_expression function: (field_expression field: (field_identifier) @call.name))
