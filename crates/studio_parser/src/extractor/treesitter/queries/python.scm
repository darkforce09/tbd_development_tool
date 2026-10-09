; Classes (bases from the superclass list)
(class_definition name: (identifier) @class.name) @class.def
(class_definition
  name: (identifier) @class.name
  superclasses: (argument_list [(identifier) (attribute)] @class.base)) @class.def

; Functions and methods
(function_definition
  name: (identifier) @func.name
  parameters: (parameters) @func.params) @func.def
(function_definition
  name: (identifier) @func.name
  parameters: (parameters) @func.params
  return_type: (_) @func.return) @func.def

; Fields: class-level assignments and self.x = ... in methods
(class_definition
  body: (block
    (expression_statement
      (assignment left: (identifier) @field.name) @field.def)))
(class_definition
  body: (block
    (expression_statement
      (assignment left: (identifier) @field.name type: (_) @field.type) @field.def)))
((assignment
  left: (attribute object: (identifier) @_self attribute: (identifier) @field.name)) @field.def
  (#eq? @_self "self"))

; Imports
(import_statement name: (dotted_name) @import.path) @import.def
(import_statement name: (aliased_import name: (dotted_name) @import.path)) @import.def
(import_from_statement module_name: (_) @import.path) @import.def
(import_from_statement module_name: (_) @import.path name: (dotted_name) @import.item) @import.def
(import_from_statement module_name: (_) @import.path name: (aliased_import name: (dotted_name) @import.item)) @import.def

; Calls
(call function: (identifier) @call.name)
(call function: (attribute attribute: (identifier) @call.name))
