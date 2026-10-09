; Classes
(class_declaration name: (_) @class.name) @class.def
(class_declaration name: (_) @class.name (class_heritage (_) @class.base)) @class.def
(class name: (_) @class.name) @class.def

; Functions
(function_declaration name: (identifier) @func.name parameters: (formal_parameters) @func.params) @func.def
(generator_function_declaration name: (identifier) @func.name parameters: (formal_parameters) @func.params) @func.def
(method_definition name: (_) @func.name parameters: (formal_parameters) @func.params) @func.def
(lexical_declaration
  (variable_declarator
    name: (identifier) @func.name
    value: [(arrow_function parameters: (formal_parameters) @func.params)
            (function_expression parameters: (formal_parameters) @func.params)])) @func.def
(lexical_declaration
  (variable_declarator
    name: (identifier) @func.name
    value: (arrow_function parameter: (identifier) @func.param))) @func.def
(variable_declaration
  (variable_declarator
    name: (identifier) @func.name
    value: [(arrow_function parameters: (formal_parameters) @func.params)
            (function_expression parameters: (formal_parameters) @func.params)])) @func.def

; Class fields
(field_definition property: (property_identifier) @field.name) @field.def

; Imports
(import_statement source: (string (string_fragment) @import.path)) @import.def
(import_statement
  (import_clause (named_imports (import_specifier name: (identifier) @import.item)))
  source: (string (string_fragment) @import.path)) @import.def
(import_statement
  (import_clause (identifier) @import.item)
  source: (string (string_fragment) @import.path)) @import.def
(import_statement
  (import_clause (namespace_import (identifier) @import.item))
  source: (string (string_fragment) @import.path)) @import.def
((call_expression
  function: (identifier) @_req
  arguments: (arguments (string (string_fragment) @import.path))) @import.def
  (#eq? @_req "require"))

; Calls
(call_expression function: (identifier) @call.name)
(call_expression function: (member_expression property: (property_identifier) @call.name))
(new_expression constructor: (identifier) @call.name)
