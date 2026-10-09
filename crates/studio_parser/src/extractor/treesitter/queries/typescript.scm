; Classes
(class_declaration name: (type_identifier) @class.name) @class.def
(class_declaration
  name: (type_identifier) @class.name
  (class_heritage (extends_clause value: (_) @class.base))) @class.def
(class_declaration
  name: (type_identifier) @class.name
  (class_heritage (implements_clause (_) @class.base))) @class.def
(abstract_class_declaration name: (type_identifier) @class.name) @class.def
(abstract_class_declaration
  name: (type_identifier) @class.name
  (class_heritage (extends_clause value: (_) @class.base))) @class.def
(class name: (type_identifier) @class.name) @class.def

; Interfaces and enums
(interface_declaration name: (type_identifier) @iface.name) @iface.def
(enum_declaration name: (identifier) @enum.name) @enum.def
(enum_declaration
  name: (identifier) @enum.name
  body: (enum_body [(property_identifier) @enum.variant
                    (enum_assignment name: (property_identifier) @enum.variant)])) @enum.def

; Functions and methods
(function_declaration name: (identifier) @func.name parameters: (formal_parameters) @func.params) @func.def
(function_declaration
  name: (identifier) @func.name
  parameters: (formal_parameters) @func.params
  return_type: (type_annotation) @func.return) @func.def
(generator_function_declaration name: (identifier) @func.name parameters: (formal_parameters) @func.params) @func.def
(method_definition name: (_) @func.name parameters: (formal_parameters) @func.params) @func.def
(method_definition
  name: (_) @func.name
  parameters: (formal_parameters) @func.params
  return_type: (type_annotation) @func.return) @func.def
(method_signature name: (_) @func.name parameters: (formal_parameters) @func.params) @func.def
(abstract_method_signature name: (_) @func.name parameters: (formal_parameters) @func.params) @func.def
(function_signature name: (identifier) @func.name parameters: (formal_parameters) @func.params) @func.def
(lexical_declaration
  (variable_declarator
    name: (identifier) @func.name
    value: [(arrow_function parameters: (formal_parameters) @func.params)
            (function_expression parameters: (formal_parameters) @func.params)])) @func.def
(lexical_declaration
  (variable_declarator
    name: (identifier) @func.name
    value: (arrow_function parameter: (identifier) @func.param))) @func.def

; Fields
(public_field_definition name: (property_identifier) @field.name) @field.def
(public_field_definition
  name: (property_identifier) @field.name
  type: (type_annotation) @field.type) @field.def
(property_signature name: (property_identifier) @field.name) @field.def

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
