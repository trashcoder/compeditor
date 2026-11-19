use std::ops::Range;
use yaml_edit::{Lang, SyntaxKind::*};

type Node = rowan::SyntaxNode<Lang>;

fn range(node: &Node) -> Range<usize> {
    let range = node.text_range();
    usize::from(range.start())..usize::from(range.end())
}

fn is_nonempty_flow(node: &Node) -> bool {
    // An empty collection has no block representation.
    matches!(
        node.first_child_or_token().map(|part| part.kind()),
        Some(LEFT_BRACE | LEFT_BRACKET)
    ) && node
        .children()
        .any(|entry| matches!(entry.kind(), MAPPING_ENTRY | SEQUENCE_ENTRY))
}

pub fn needs_formatting(node: &Node) -> bool {
    node.descendants().any(|node| is_nonempty_flow(&node))
}

pub fn block_style(source: &str, node: &Node) -> String {
    // A nested flow collection can only become a block with its enclosing flow.
    let enclosing = node.ancestors().skip(1).filter(is_nonempty_flow).last();
    let node = enclosing.as_ref().unwrap_or(node);
    let mut patches = Vec::new();
    collect_patches(source, node, &mut patches);
    let mut output = source.to_owned();
    for (span, replacement) in patches.into_iter().rev() {
        output.replace_range(span, &replacement);
    }
    output
}

fn collect_patches(source: &str, node: &Node, patches: &mut Vec<(Range<usize>, String)>) {
    if !is_nonempty_flow(node) {
        for child in node.children() {
            collect_patches(source, &child, patches);
        }
        return;
    }

    let mut span = range(node);
    let owner = node
        .ancestors()
        .skip(1)
        .find(|parent| matches!(parent.kind(), MAPPING_ENTRY | SEQUENCE_ENTRY | DOCUMENT))
        .expect("collection belongs to a parsed document");
    let owner_start = range(&owner).start;
    let owner_line_start = source[..owner_start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let owner_indent = if owner.kind() == DOCUMENT {
        0
    } else {
        source[owner_line_start..owner_start]
            .trim_start_matches('\u{feff}')
            .len()
    };
    let indent = owner_indent + if owner.kind() == DOCUMENT { 0 } else { 2 };

    let line_start = source[..span.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let prefix = &source[line_start..span.start];
    let standalone = prefix.trim_start_matches('\u{feff}').trim().is_empty();
    let mut replacement = String::new();
    span.start -= prefix.len() - prefix.trim_end_matches([' ', '\t']).len();
    if !standalone {
        replacement.push('\n');
    }

    render_collection(node, indent, &mut replacement);
    replacement.pop(); // The source still contains the newline after the collection.

    let suffix = &source[span.end..];
    let after_spacing = suffix.trim_start_matches([' ', '\t']);
    if after_spacing.starts_with('#') || after_spacing.starts_with(':') {
        // A trailing comment or explicit key separator needs its own line after expansion.
        span.end += suffix.len() - after_spacing.len();
        replacement.push('\n');
        pad(&mut replacement, owner_indent);
    }
    patches.push((span, replacement));
}

fn render_collection(node: &Node, indent: usize, output: &mut String) {
    for part in node.children_with_tokens() {
        match part.as_node() {
            Some(entry) if entry.kind() == MAPPING_ENTRY => {
                let key = entry.children().find(|node| node.kind() == KEY);
                let value = entry.children().find(|node| node.kind() == VALUE);
                let explicit = key.as_ref().is_none_or(|key| {
                    key.text().is_empty()
                        || key.to_string().contains('\n')
                        || key
                            .descendants()
                            .any(|node| matches!(node.kind(), MAPPING | SEQUENCE))
                        || key
                            .children_with_tokens()
                            .any(|part| part.kind() == QUESTION)
                });
                pad(output, indent);
                if explicit {
                    output.push('?');
                    render_value(key.as_ref(), indent, output);
                    output.push('\n');
                    pad(output, indent);
                } else if let Some(key) = &key {
                    render_content(key, indent, output);
                }
                output.push(':');
                render_value(value.as_ref(), indent, output);
                output.push('\n');
                render_comments(entry, indent, output);
            }
            Some(entry) if entry.kind() == SEQUENCE_ENTRY => {
                pad(output, indent);
                output.push('-');
                render_value(Some(entry), indent, output);
                output.push('\n');
                render_comments(entry, indent, output);
            }
            _ => {}
        }
        if let Some(token) = part.into_token().filter(|token| token.kind() == COMMENT) {
            pad(output, indent);
            output.push_str(token.text());
            output.push('\n');
        }
    }
}

fn render_value(node: Option<&Node>, indent: usize, output: &mut String) {
    if let Some(node) = node {
        let start = output.len();
        output.push(' ');
        render_content(node, indent, output);
        if output.len() == start + 1 {
            output.pop();
        }
    }
}

fn render_content(node: &Node, indent: usize, output: &mut String) {
    match node.kind() {
        MAPPING | SEQUENCE if node.children().next().is_some() => {
            if output.ends_with(' ') {
                output.pop();
            }
            output.push('\n');
            render_collection(node, indent + 2, output);
            output.pop();
        }
        KEY | VALUE | SEQUENCE_ENTRY | TAGGED_NODE => {
            for part in node.children_with_tokens() {
                if let Some(child) = part.as_node() {
                    render_content(child, indent, output);
                } else if let Some(token) = part
                    .as_token()
                    .filter(|token| matches!(token.kind(), ANCHOR | TAG))
                {
                    output.push_str(token.text());
                    output.push(' ');
                }
            }
            if output.ends_with(' ') && node.children().next().is_none() {
                // Properties without a value belong to an implicit null.
                output.pop();
            }
        }
        _ => {
            let text = node.to_string();
            let mut lines = text.split('\n');
            output.push_str(lines.next().unwrap_or_default());
            for line in lines {
                output.push('\n');
                pad(output, indent + 2);
                output.push_str(line.trim_start_matches([' ', '\t']));
            }
        }
    }
}

fn render_comments(node: &Node, indent: usize, output: &mut String) {
    for token in node
        .children_with_tokens()
        .filter_map(|part| part.into_token())
    {
        if token.kind() == COMMENT {
            pad(output, indent);
            output.push_str(token.text());
            output.push('\n');
        }
    }
}

fn pad(output: &mut String, indent: usize) {
    output.extend(std::iter::repeat_n(' ', indent));
}
