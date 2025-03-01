use super::*;

pub trait Visitor {
    fn visit_source_unit(&mut self, _source_unit: &SourceUnit) {}
    fn visit_import_directive(&mut self, _directive: &ImportDirective) {}
    fn visit_pragma_directive(&mut self, _directive: &PragmaDirective) {}
    fn visit_block(&mut self, _block: &Block) {}
    fn visit_statement(&mut self, _statement: &Statement) {}
    fn visit_expression(&mut self, _expression: &Expression) {}
    fn visit_function_call(&mut self, _function_call: &FunctionCall) {}
    fn visit_user_defined_type_name(&mut self, _type_name: &UserDefinedTypeName) {}
    fn visit_identifier_path(&mut self, _identifier_path: &IdentifierPath) {}
    fn visit_type_name(&mut self, _type_name: &TypeName) {}
    fn visit_parameter_list(&mut self, _parameter_list: &ParameterList) {}
    fn visit_function_definition(&mut self, _definition: &FunctionDefinition) {}
    fn visit_enum_definition(&mut self, _definition: &EnumDefinition) {}
    fn visit_error_definition(&mut self, _definition: &ErrorDefinition) {}
    fn visit_event_definition(&mut self, _definition: &EventDefinition) {}
    fn visit_struct_definition(&mut self, _definition: &StructDefinition) {}
    fn visit_modifier_definition(&mut self, _definition: &ModifierDefinition) {}
    fn visit_variable_declaration(&mut self, _declaration: &VariableDeclaration) {}
    fn visit_overrides(&mut self, _specifier: &OverrideSpecifier) {}
    fn visit_user_defined_value_type(&mut self, _value_type: &UserDefinedValueTypeDefinition) {}
    fn visit_contract_definition(&mut self, _definition: &ContractDefinition) {}
    fn visit_using_for(&mut self, _directive: &UsingForDirective) {}
    fn visit_unary_operation(&mut self, _unary_op: &UnaryOperation) {}
    fn visit_binary_operation(&mut self, _binary_op: &BinaryOperation) {}
    fn visit_conditional(&mut self, _conditional: &Conditional) {}
    fn visit_tuple_expression(&mut self, _tuple_expression: &TupleExpression) {}
    fn visit_new_expression(&mut self, _new_expression: &NewExpression) {}
    fn visit_assignment(&mut self, _assignment: &Assignment) {}
    fn visit_identifier(&mut self, _identifier: &Identifier) {}
    fn visit_index_access(&mut self, _index_access: &IndexAccess) {}
    fn visit_index_range_access(&mut self, _index_range_access: &IndexRangeAccess) {}
    fn visit_while_statement(&mut self, _while_statement: &WhileStatement) {}
    fn visit_for_statement(&mut self, _for_statement: &ForStatement) {}
    fn visit_if_statement(&mut self, _if_statement: &IfStatement) {}
    fn visit_do_while_statement(&mut self, _do_while_statement: &DoWhileStatement) {}
    fn visit_emit_statement(&mut self, _emit_statement: &EmitStatement) {}
    fn visit_unchecked_block(&mut self, _unchecked_block: &UncheckedBlock) {}
    fn visit_try_statement(&mut self, _try_statement: &TryStatement) {}
    fn visit_revert_statement(&mut self, _revert_statement: &RevertStatement) {}
    fn visit_member_access(&mut self, _member_access: &MemberAccess) {}
    fn visit_mapping(&mut self, _mapping: &Mapping) {}
    fn visit_elementary_type_name(&mut self, _elementary_type_name: &ElementaryTypeName) {}
    fn visit_literal(&mut self, _literal: &Literal) {}
    fn visit_function_type_name(&mut self, _function_type_name: &FunctionTypeName) {}
    fn visit_array_type_name(&mut self, _array_type_name: &ArrayTypeName) {}
    fn visit_function_call_options(&mut self, _function_call: &FunctionCallOptions) {}
    fn visit_return(&mut self, _return: &Return) {}
    fn visit_inheritance_specifier(&mut self, _specifier: &InheritanceSpecifier) {}
    fn visit_modifier_invocation(&mut self, _invocation: &ModifierInvocation) {}
    fn visit_inline_assembly(&mut self, _assembly: &InlineAssembly) {}
    fn visit_external_assembly_reference(&mut self, _ref: &ExternalInlineAssemblyReference) {}
}

pub trait Walk {
    fn walk(&self, visitor: &mut dyn Visitor);
}

macro_rules! impl_walk {
    // Implement `Walk` for a type, calling the given function.
    ($ty:ty, | $val:ident, $visitor:ident | $e:expr) => {
        impl Walk for $ty {
            fn walk(&self, visitor: &mut dyn Visitor) {
                let $val = self;
                let $visitor = visitor;
                $e
            }
        }
    };
    ($ty:ty, $func:ident) => {
        impl_walk!($ty, |obj, visitor| {
            visitor.$func(obj);
        });
    };
    ($ty:ty, $func:ident, | $val:ident, $visitor:ident | $e:expr) => {
        impl_walk!($ty, |$val, $visitor| {
            $visitor.$func($val);
            $e
        });
    };
}

impl_walk!(SourceUnit, visit_source_unit, |source_unit, visitor| {
    source_unit.nodes.iter().for_each(|part| {
        part.walk(visitor);
    });
});

impl_walk!(SourceUnitPart, |part, visitor| {
    match part {
        SourceUnitPart::ContractDefinition(contract) => {
            contract.walk(visitor);
        }
        SourceUnitPart::UsingForDirective(directive) => {
            directive.walk(visitor);
        }
        SourceUnitPart::ErrorDefinition(error) => {
            error.walk(visitor);
        }
        SourceUnitPart::EventDefinition(event) => {
            event.walk(visitor);
        }
        SourceUnitPart::StructDefinition(struct_) => {
            struct_.walk(visitor);
        }
        SourceUnitPart::VariableDeclaration(declaration) => {
            declaration.walk(visitor);
        }
        SourceUnitPart::FunctionDefinition(function) => {
            function.walk(visitor);
        }
        SourceUnitPart::UserDefinedValueTypeDefinition(value_type) => {
            value_type.walk(visitor);
        }
        SourceUnitPart::ImportDirective(directive) => {
            directive.walk(visitor);
        }
        SourceUnitPart::EnumDefinition(enum_) => {
            enum_.walk(visitor);
        }
        SourceUnitPart::PragmaDirective(directive) => {
            directive.walk(visitor);
        }
    }
});

impl_walk!(ContractDefinition, visit_contract_definition, |contract, visitor| {
    contract.base_contracts.iter().for_each(|base_contract| {
        base_contract.walk(visitor);
    });

    for part in &contract.nodes {
        match part {
            ContractDefinitionPart::FunctionDefinition(function) => {
                function.walk(visitor);
            }
            ContractDefinitionPart::ErrorDefinition(error) => {
                error.walk(visitor);
            }
            ContractDefinitionPart::EventDefinition(event) => {
                event.walk(visitor);
            }
            ContractDefinitionPart::StructDefinition(struct_) => {
                struct_.walk(visitor);
            }
            ContractDefinitionPart::VariableDeclaration(declaration) => {
                declaration.walk(visitor);
            }
            ContractDefinitionPart::ModifierDefinition(modifier) => {
                modifier.walk(visitor);
            }
            ContractDefinitionPart::UserDefinedValueTypeDefinition(definition) => {
                definition.walk(visitor);
            }
            ContractDefinitionPart::UsingForDirective(directive) => {
                directive.walk(visitor);
            }
            ContractDefinitionPart::EnumDefinition(enum_) => {
                enum_.walk(visitor);
            }
        }
    }
});

impl_walk!(Expression, visit_expression, |expr, visitor| {
    match expr {
        Expression::FunctionCall(expression) => {
            expression.walk(visitor);
        }
        Expression::MemberAccess(member_access) => {
            member_access.walk(visitor);
        }
        Expression::IndexAccess(index_access) => {
            index_access.walk(visitor);
        }
        Expression::UnaryOperation(unary_op) => {
            unary_op.walk(visitor);
        }
        Expression::BinaryOperation(expression) => {
            expression.walk(visitor);
        }
        Expression::Conditional(expression) => {
            expression.walk(visitor);
        }
        Expression::TupleExpression(tuple) => {
            tuple.walk(visitor);
        }
        Expression::NewExpression(expression) => {
            expression.walk(visitor);
        }
        Expression::Assignment(expression) => {
            expression.walk(visitor);
        }
        Expression::Identifier(identifier) => {
            identifier.walk(visitor);
        }
        Expression::FunctionCallOptions(function_call) => {
            function_call.walk(visitor);
        }
        Expression::IndexRangeAccess(range_access) => {
            range_access.walk(visitor);
        }
        Expression::Literal(literal) => {
            literal.walk(visitor);
        }
        Expression::ElementaryTypeNameExpression(type_name) => {
            type_name.walk(visitor);
        }
    }
});

impl_walk!(Statement, visit_statement, |statement, visitor| {
    match statement {
        Statement::Block(block) => {
            block.walk(visitor);
        }
        Statement::WhileStatement(statement) => {
            statement.walk(visitor);
        }
        Statement::ForStatement(statement) => {
            statement.walk(visitor);
        }
        Statement::IfStatement(statement) => {
            statement.walk(visitor);
        }
        Statement::DoWhileStatement(statement) => {
            statement.walk(visitor);
        }
        Statement::EmitStatement(statement) => {
            statement.walk(visitor);
        }
        Statement::VariableDeclarationStatement(statement) => {
            statement.walk(visitor);
        }
        Statement::ExpressionStatement(statement) => {
            statement.walk(visitor);
        }
        Statement::UncheckedBlock(statement) => {
            statement.walk(visitor);
        }
        Statement::TryStatement(statement) => {
            statement.walk(visitor);
        }
        Statement::RevertStatement(statement) => {
            statement.walk(visitor);
        }
        Statement::Return(statement) => {
            statement.walk(visitor);
        }
        Statement::InlineAssembly(assembly) => {
            assembly.walk(visitor);
        }
        Statement::Break(_) | Statement::Continue(_) | Statement::PlaceholderStatement(_) => {}
    }
});

impl_walk!(FunctionDefinition, visit_function_definition, |function, visitor| {
    function.parameters.walk(visitor);
    function.return_parameters.walk(visitor);

    if let Some(overrides) = &function.overrides {
        overrides.walk(visitor);
    }

    if let Some(body) = &function.body {
        body.walk(visitor);
    }

    function.modifiers.iter().for_each(|m| m.walk(visitor));
});

impl_walk!(ErrorDefinition, visit_error_definition, |error, visitor| {
    error.parameters.walk(visitor);
});

impl_walk!(EventDefinition, visit_event_definition, |event, visitor| {
    event.parameters.walk(visitor);
});

impl_walk!(StructDefinition, visit_struct_definition, |struct_, visitor| {
    struct_.members.iter().for_each(|member| member.walk(visitor));
});

impl_walk!(ModifierDefinition, visit_modifier_definition, |modifier, visitor| {
    if let Some(body) = &modifier.body {
        body.walk(visitor);
    }
    if let Some(override_) = &modifier.overrides {
        override_.walk(visitor);
    }
    modifier.parameters.walk(visitor);
});

impl_walk!(VariableDeclaration, visit_variable_declaration, |declaration, visitor| {
    if let Some(value) = &declaration.value {
        value.walk(visitor);
    }

    if let Some(type_name) = &declaration.type_name {
        type_name.walk(visitor);
    }
});

impl_walk!(OverrideSpecifier, visit_overrides, |override_, visitor| {
    override_.overrides.iter().for_each(|type_name| {
        type_name.walk(visitor);
    });
});

impl_walk!(UserDefinedValueTypeDefinition, visit_user_defined_value_type, |value_type, visitor| {
    value_type.underlying_type.walk(visitor);
});

impl_walk!(FunctionCallOptions, visit_function_call_options, |function_call, visitor| {
    function_call.expression.walk(visitor);
    function_call.options.iter().for_each(|option| {
        option.walk(visitor);
    });
});

impl_walk!(Return, visit_return, |return_, visitor| {
    if let Some(expr) = return_.expression.as_ref() {
        expr.walk(visitor);
    }
});

impl_walk!(UsingForDirective, visit_using_for, |directive, visitor| {
    if let Some(type_name) = &directive.type_name {
        type_name.walk(visitor);
    }
    if let Some(library_name) = &directive.library_name {
        library_name.walk(visitor);
    }
    for function in &directive.function_list {
        function.walk(visitor);
    }
});

impl_walk!(UnaryOperation, visit_unary_operation, |unary_op, visitor| {
    unary_op.sub_expression.walk(visitor);
});

impl_walk!(BinaryOperation, visit_binary_operation, |binary_op, visitor| {
    binary_op.lhs.walk(visitor);
    binary_op.rhs.walk(visitor);
});

impl_walk!(Conditional, visit_conditional, |conditional, visitor| {
    conditional.condition.walk(visitor);
    conditional.true_expression.walk(visitor);
    conditional.false_expression.walk(visitor);
});

impl_walk!(TupleExpression, visit_tuple_expression, |tuple_expression, visitor| {
    tuple_expression.components.iter().filter_map(|component| component.as_ref()).for_each(
        |component| {
            component.walk(visitor);
        },
    );
});

impl_walk!(NewExpression, visit_new_expression, |new_expression, visitor| {
    new_expression.type_name.walk(visitor);
});

impl_walk!(Assignment, visit_assignment, |assignment, visitor| {
    assignment.lhs.walk(visitor);
    assignment.rhs.walk(visitor);
});
impl_walk!(IfStatement, visit_if_statement, |if_statement, visitor| {
    if_statement.condition.walk(visitor);
    if_statement.true_body.walk(visitor);

    if let Some(false_body) = &if_statement.false_body {
        false_body.walk(visitor);
    }
});

impl_walk!(IndexAccess, visit_index_access, |index_access, visitor| {
    index_access.base_expression.walk(visitor);
    if let Some(index_expression) = &index_access.index_expression {
        index_expression.walk(visitor);
    }
});

impl_walk!(IndexRangeAccess, visit_index_range_access, |index_range_access, visitor| {
    index_range_access.base_expression.walk(visitor);
    if let Some(start_expression) = &index_range_access.start_expression {
        start_expression.walk(visitor);
    }
    if let Some(end_expression) = &index_range_access.end_expression {
        end_expression.walk(visitor);
    }
});

impl_walk!(WhileStatement, visit_while_statement, |while_statement, visitor| {
    while_statement.condition.walk(visitor);
    while_statement.body.walk(visitor);
});

impl_walk!(ForStatement, visit_for_statement, |for_statement, visitor| {
    for_statement.body.walk(visitor);
    if let Some(condition) = &for_statement.condition {
        condition.walk(visitor);
    }

    if let Some(loop_expression) = &for_statement.loop_expression {
        loop_expression.walk(visitor);
    }

    if let Some(initialization_expr) = &for_statement.initialization_expression {
        initialization_expr.walk(visitor);
    }
});

impl_walk!(DoWhileStatement, visit_do_while_statement, |do_while_statement, visitor| {
    do_while_statement.body.walk(visitor);
    do_while_statement.condition.walk(visitor);
});

impl_walk!(EmitStatement, visit_emit_statement, |emit_statement, visitor| {
    emit_statement.event_call.walk(visitor);
});

impl_walk!(VariableDeclarationStatement, |stmt, visitor| {
    stmt.declarations.iter().filter_map(|d| d.as_ref()).for_each(|declaration| {
        declaration.walk(visitor);
    });
    if let Some(initial_value) = &stmt.initial_value {
        initial_value.walk(visitor);
    }
});

impl_walk!(UncheckedBlock, visit_unchecked_block, |unchecked_block, visitor| {
    unchecked_block.statements.iter().for_each(|statement| {
        statement.walk(visitor);
    });
});

impl_walk!(TryStatement, visit_try_statement, |try_statement, visitor| {
    try_statement.clauses.iter().for_each(|clause| {
        clause.block.walk(visitor);

        if let Some(parameter_list) = &clause.parameters {
            parameter_list.walk(visitor);
        }
    });

    try_statement.external_call.walk(visitor);
});

impl_walk!(RevertStatement, visit_revert_statement, |revert_statement, visitor| {
    revert_statement.error_call.walk(visitor);
});

impl_walk!(MemberAccess, visit_member_access, |member_access, visitor| {
    member_access.expression.walk(visitor);
});

impl_walk!(FunctionCall, visit_function_call, |function_call, visitor| {
    function_call.expression.walk(visitor);
    function_call.arguments.iter().for_each(|argument| {
        argument.walk(visitor);
    });
});

impl_walk!(Block, visit_block, |block, visitor| {
    block.statements.iter().for_each(|statement| {
        statement.walk(visitor);
    });
});

impl_walk!(UserDefinedTypeName, visit_user_defined_type_name, |type_name, visitor| {
    if let Some(path_node) = &type_name.path_node {
        path_node.walk(visitor);
    }
});

impl_walk!(TypeName, visit_type_name, |type_name, visitor| {
    match type_name {
        TypeName::ElementaryTypeName(type_name) => {
            type_name.walk(visitor);
        }
        TypeName::UserDefinedTypeName(type_name) => {
            type_name.walk(visitor);
        }
        TypeName::Mapping(mapping) => {
            mapping.walk(visitor);
        }
        TypeName::ArrayTypeName(array) => {
            array.walk(visitor);
        }
        TypeName::FunctionTypeName(function) => {
            function.walk(visitor);
        }
    }
});

impl_walk!(FunctionTypeName, visit_function_type_name, |function, visitor| {
    function.parameter_types.walk(visitor);
    function.return_parameter_types.walk(visitor);
});

impl_walk!(ParameterList, visit_parameter_list, |parameter_list, visitor| {
    parameter_list.parameters.iter().for_each(|parameter| {
        parameter.walk(visitor);
    });
});

impl_walk!(Mapping, visit_mapping, |mapping, visitor| {
    mapping.key_type.walk(visitor);
    mapping.value_type.walk(visitor);
});

impl_walk!(ArrayTypeName, visit_array_type_name, |array, visitor| {
    array.base_type.walk(visitor);
    if let Some(length) = &array.length {
        length.walk(visitor);
    }
});

impl_walk!(InheritanceSpecifier, visit_inheritance_specifier, |specifier, visitor| {
    specifier.base_name.walk(visitor);
    specifier.arguments.iter().for_each(|arg| {
        arg.walk(visitor);
    });
});

impl_walk!(ModifierInvocation, visit_modifier_invocation, |invocation, visitor| {
    invocation.arguments.iter().for_each(|arg| arg.walk(visitor));
    invocation.modifier_name.walk(visitor);
});

impl_walk!(InlineAssembly, visit_inline_assembly, |assembly, visitor| {
    assembly.external_references.iter().for_each(|reference| {
        reference.walk(visitor);
    });
});

impl_walk!(ExternalInlineAssemblyReference, visit_external_assembly_reference);

impl_walk!(ElementaryTypeName, visit_elementary_type_name);
impl_walk!(Literal, visit_literal);
impl_walk!(ImportDirective, visit_import_directive);
impl_walk!(PragmaDirective, visit_pragma_directive);
impl_walk!(IdentifierPath, visit_identifier_path);
impl_walk!(EnumDefinition, visit_enum_definition);
impl_walk!(Identifier, visit_identifier);

impl_walk!(UserDefinedTypeNameOrIdentifierPath, |type_name, visitor| {
    match type_name {
        UserDefinedTypeNameOrIdentifierPath::UserDefinedTypeName(type_name) => {
            type_name.walk(visitor);
        }
        UserDefinedTypeNameOrIdentifierPath::IdentifierPath(identifier_path) => {
            identifier_path.walk(visitor);
        }
    }
});

impl_walk!(BlockOrStatement, |block_or_statement, visitor| {
    match block_or_statement {
        BlockOrStatement::Block(block) => {
            block.walk(visitor);
        }
        BlockOrStatement::Statement(statement) => {
            statement.walk(visitor);
        }
    }
});

impl_walk!(ExpressionOrVariableDeclarationStatement, |val, visitor| {
    match val {
        ExpressionOrVariableDeclarationStatement::ExpressionStatement(expression) => {
            expression.walk(visitor);
        }
        ExpressionOrVariableDeclarationStatement::VariableDeclarationStatement(stmt) => {
            stmt.walk(visitor);
        }
    }
});

impl_walk!(IdentifierOrIdentifierPath, |val, visitor| {
    match val {
        IdentifierOrIdentifierPath::Identifier(ident) => {
            ident.walk(visitor);
        }
        IdentifierOrIdentifierPath::IdentifierPath(path) => {
            path.walk(visitor);
        }
    }
});

impl_walk!(ExpressionStatement, |expression_statement, visitor| {
    expression_statement.expression.walk(visitor);
});

impl_walk!(ElementaryTypeNameExpression, |type_name, visitor| {
    type_name.type_name.walk(visitor);
});

impl_walk!(ElementaryOrRawTypeName, |type_name, visitor| {
    match type_name {
        ElementaryOrRawTypeName::ElementaryTypeName(type_name) => {
            type_name.walk(visitor);
        }
        ElementaryOrRawTypeName::Raw(_) => {}
    }
});

impl_walk!(UsingForFunctionItem, |item, visitor| {
    match item {
        UsingForFunctionItem::Function(func) => {
            func.function.walk(visitor);
        }
        UsingForFunctionItem::OverloadedOperator(operator) => {
            operator.walk(visitor);
        }
    }
});

impl_walk!(OverloadedOperator, |operator, visitor| {
    operator.definition.walk(visitor);
});
