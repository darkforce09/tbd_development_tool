; Classes and structs
(class_specifier name: (type_identifier) @class.name body: (field_declaration_list)) @class.def
(struct_specifier name: (type_identifier) @class.name body: (field_declaration_list)) @class.def
(class_specifier
  name: (type_identifier) @class.name
  (base_class_clause [(type_identifier) (qualified_identifier) (template_type)] @class.base)) @class.def
(struct_specifier
  name: (type_identifier) @class.name
  (base_class_clause [(type_identifier) (qualified_identifier) (template_type)] @class.base)) @class.def
(type_definition
  type: (struct_specifier body: (field_declaration_list))
  declarator: (type_identifier) @class.name) @class.def
(field_declaration type: (_) @field.type declarator: (field_identifier) @field.name) @field.def
(field_declaration type: (_) @field.type declarator: (pointer_declarator declarator: (field_identifier) @field.name)) @field.def
(field_declaration type: (_) @field.type declarator: (reference_declarator (field_identifier) @field.name)) @field.def

; Enums
(enum_specifier name: (type_identifier) @enum.name body: (enumerator_list)) @enum.def
(enum_specifier
  name: (type_identifier) @enum.name
  body: (enumerator_list (enumerator name: (identifier) @enum.variant))) @enum.def

; Functions, methods, constructors (out-of-line `Type::method` attaches to Type)
(function_definition
  declarator: (function_declarator
    declarator: [(identifier) (field_identifier) (destructor_name) (operator_name)] @func.name
    parameters: (parameter_list) @func.params)) @func.def
(function_definition
  type: (_) @func.return
  declarator: (function_declarator
    declarator: [(identifier) (field_identifier) (operator_name)] @func.name
    parameters: (parameter_list) @func.params)) @func.def
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      scope: (namespace_identifier) @func.receiver
      name: [(identifier) (destructor_name) (operator_name)] @func.name)
    parameters: (parameter_list) @func.params)) @func.def
(function_definition
  type: (_) @func.return
  declarator: (function_declarator
    declarator: (qualified_identifier
      scope: (namespace_identifier) @func.receiver
      name: [(identifier) (operator_name)] @func.name)
    parameters: (parameter_list) @func.params)) @func.def
(function_definition
  type: (_) @func.return
  declarator: [(pointer_declarator (function_declarator
                 declarator: [(identifier) (field_identifier)] @func.name
                 parameters: (parameter_list) @func.params))
               (reference_declarator (function_declarator
                 declarator: [(identifier) (field_identifier)] @func.name
                 parameters: (parameter_list) @func.params))]) @func.def

; Includes
(preproc_include path: (string_literal (string_content) @import.path)) @import.def
(preproc_include path: (system_lib_string) @import.path) @import.def

; Calls
(call_expression function: (identifier) @call.name)
(call_expression function: (field_expression field: (field_identifier) @call.name))
(call_expression function: (qualified_identifier name: (identifier) @call.name))
