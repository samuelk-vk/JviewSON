extends TextEdit

const STRING_COLOR = Color(0.62, 0.9, 0.55)
const NUMBER_COLOR = Color(0.98, 0.74, 0.31)
const SYMBOL_COLOR = Color(0.86, 0.9, 0.95)
const BOOLEAN_COLOR = Color(0.45, 0.8, 1.0)
const NULL_COLOR = Color(0.95, 0.48, 0.75)

func _ready() -> void:
	# Use explicit high-contrast colors so values remain readable on dark backgrounds.
	var highlighter := CodeHighlighter.new()
	highlighter.number_color = NUMBER_COLOR
	highlighter.symbol_color = SYMBOL_COLOR
	highlighter.add_color_region('"', '"', STRING_COLOR)
	highlighter.add_keyword_color("true", BOOLEAN_COLOR)
	highlighter.add_keyword_color("false", BOOLEAN_COLOR)
	highlighter.add_keyword_color("null", NULL_COLOR)
	syntax_highlighter = highlighter
	

func set_unformated_text(unformated_text: String):
	text = unformated_text
