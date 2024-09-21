use bitflags::bitflags;
use kurbo::Rect;
use std::borrow::Cow;
use std::cell::OnceCell;
use std::{fmt, slice};
use std::ops::{Deref, Range};
use std::sync::Arc;

use skia_safe as sk;
use skia_safe::font_style::{Weight, Width};
use skia_safe::textlayout::{FontCollection, RectHeightStyle, RectWidthStyle};
use skia_safe::{FontMgr, FontStyle};
use tracy_client::span;

use crate::drawing::{FromSkia, ToSkia};
use crate::style::{style_properties, Style};
use crate::Color;

thread_local! {
    static FONT_COLLECTION: OnceCell<FontCollection> = OnceCell::new();
}

/// Returns the FontCollection for the current thread.
///
/// FontCollections (and other objects that reference them, e.g. Paragraph)
/// are bound to the thread in which they were created.
pub(crate) fn get_font_collection() -> FontCollection {
    // Ideally I'd like to have only one font collection for all threads.
    // However, FontCollection isn't Send or Sync, and `Paragraphs` hold a reference to a FontCollection,
    // so, to be able to create Paragraphs from different threads, there must be one FontCollection
    // per thread.
    //
    // See also https://github.com/rust-skia/rust-skia/issues/537
    FONT_COLLECTION.with(|fc| {
        fc.get_or_init(|| {
            let mut font_collection = FontCollection::new();
            font_collection.set_default_font_manager(FontMgr::new(), None);
            font_collection
        })
        .clone()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomFontAxisValue(pub u32);

bitflags! {
    #[derive(Copy,Clone)]
    pub struct TextStyleFlags: u32 {
        const FONT_FAMILY = 1;
        const FONT_SIZE = 2;
        const FONT_WEIGHT = 4;
        const FONT_ITALIC = 8;
        const FONT_OBLIQUE = 16;
        const FONT_WIDTH = 32;
        const TEXT_COLOR = 64;
    }
}

impl Default for TextStyleFlags {
    fn default() -> Self {
        TextStyleFlags::empty()
    }
}

/// Describes the style of a text run.
///
/// It doesn't necessarily specify all the properties of the text. Unspecified properties
/// are inherited from the parent style.
#[derive(Clone)]
pub struct TextStyle<'a> {
    pub font_family: Cow<'a, str>,
    pub font_size: f64,
    pub font_weight: i32,
    pub font_italic: bool,
    pub font_oblique: bool,
    pub font_width: i32,
    pub color: Color,
}

impl Default for TextStyle<'static> {
    fn default() -> Self {
        TextStyle::new().into_static()
    }
}

impl<'a> TextStyle<'a> {
    pub fn new() -> TextStyle<'a> {
        TextStyle {
            font_family: Cow::Borrowed("Inter Display"),
            font_size: 16.0,
            font_weight: 400,
            font_italic: false,
            font_oblique: false,
            font_width: *Width::NORMAL,
            color: Color::from_rgb_u8(0, 0, 0),
        }
    }

    pub fn font_family(mut self, font_family: impl Into<Cow<'a, str>>) -> Self {
        self.font_family = font_family.into();
        self
    }
    pub fn font_size(mut self, font_size: f64) -> Self {
        self.font_size = font_size;
        self
    }

    pub fn font_weight(mut self, font_weight: i32) -> Self {
        self.font_weight = font_weight;
        self
    }

    pub fn font_italic(mut self, font_italic: bool) -> Self {
        self.font_italic = font_italic;
        self
    }

    pub fn font_oblique(mut self, font_oblique: bool) -> Self {
        self.font_oblique = font_oblique;
        self
    }

    pub fn font_width(mut self, font_width: i32) -> Self {
        self.font_width = font_width;
        self
    }

    pub fn color(mut self, text_color: Color) -> Self {
        self.color = text_color;
        self
    }

    pub fn into_static(self) -> TextStyle<'static> {
        TextStyle {
            font_family: Cow::Owned(self.font_family.into_owned()),
            font_size: self.font_size,
            font_weight: self.font_weight,
            font_italic: self.font_italic,
            font_oblique: self.font_oblique,
            font_width: self.font_width,
            color: self.color,
        }
    }

    pub(crate) fn to_skia(&self) -> skia_safe::textlayout::TextStyle {
        let mut sk_style = sk::textlayout::TextStyle::new();
        sk_style.set_font_families(&[self.font_family.as_ref()]);
        sk_style.set_font_size(self.font_size as sk::scalar);
        let slant = if self.font_italic {
            sk::font_style::Slant::Italic
        } else if self.font_oblique {
            sk::font_style::Slant::Oblique
        } else {
            sk::font_style::Slant::Upright
        };
        sk_style.set_font_style(FontStyle::new(self.font_weight.into(), self.font_width.into(), slant));
        sk_style.set_color(self.color.to_skia().to_color());
        sk_style
    }
}

/// String slice with associated styling properties.
#[derive(Copy, Clone)]
pub struct AttributedRange<'a> {
    pub str: &'a str,
    pub style: &'a TextStyle<'a>,
}

/*/// Type of values produced by the `text!` macro.
#[derive(Copy, Clone)]
pub struct AttributedStr<'a> {
    pub runs: &'a [AttributedRange<'a>],
}*/

pub type AttributedStr<'a> = [AttributedRange<'a>];

#[doc(hidden)]
pub fn cow_format_args(args: fmt::Arguments) -> Cow<str> {
    match args.as_str() {
        Some(s) => Cow::Borrowed(s),
        None => Cow::Owned(args.to_string()),
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __text {
    // Parse styles
    (@style($s:ident) rgb ($($p:expr),*) ) => {
        $s.color = $crate::Color::from_rgb_u8($($p),*);
    };

    (@style($s:ident) hexcolor ($f:expr) ) => {
        $s.color = $crate::Color::from_hex($f);
    };

    (@style($s:ident) i ) => {
        $s.font_italic = true;
    };

    (@style($s:ident) b ) => {
        $s.font_weight = 700;
    };

    (@style($s:ident) family ($f:expr) ) => {
        $s.font_family = $f.into();
    };

    (@style($s:ident) size ($f:expr) ) => {
        $s.font_size = $f;
    };

    (@style($s:ident) weight ($f:expr) ) => {
        $s.font_weight = $f;
    };

    (@style($s:ident) width ($f:expr) ) => {
        $s.font_width = $f;
    };

    (@style($s:ident) oblique ) => {
        $s.font_oblique = true;
    };

    (@style($s:ident) style ($f:expr) ) => {
        $s = $f.clone();
    };

    (@style($s:ident) $($rest:tt)*) => {
        compile_error!("Unrecognized style property");
    };

    /////////////////////////////////
    // style stack reverser
    (@apply_styles($s:ident) ) => {};

    (@apply_styles($s:ident) ( $(($($styles:tt)*))* ) $($rest:tt)* ) => {
         $crate::__text!(@apply_styles($s) $($rest)*);
         $($crate::__text!(@style($s) $($styles)*);)*
    };

    ////////////////////
    // finish rule
    (
        // input
        ()
        // output
        ($($sty:tt)*)
        //($strlen:expr)
        ($(
            ($string:literal, $($styles:tt)* )
        )*)
    ) => {
        [
            $(
            $crate::text::AttributedRange {
                str: &*$crate::text::cow_format_args(::std::format_args!($string)),
                style: &{
                    let mut __s = $crate::text::TextStyle::default();
                    $crate::__text!(@apply_styles(__s) $($styles)*);
                    __s
                }
            },
            )*
        ]
    };

    ////////////////////
    // pop style
    (
        // input
        ( @pop $($rest:tt)* )
        // output
        ( ($($sty_top:tt)*) $(($($sty_rest:tt)*))* )
        ($($ranges:tt)*)
    ) => {
        $crate::__text!(
            ($($rest)*)
            ($(($($sty_rest)*))*)
            ($($ranges)*)
        )
    };

    ////////////////////
    // string literal
    (
        // input
        ( $str:literal $($rest:tt)* )
        // output
        ( $(($($sty:tt)*))*)
        ( $($ranges:tt)* )

    ) => {
        $crate::__text!(
            ($($rest)*)
            ( $(($($sty)*))*)
            ( $($ranges)* ( $str, $(($($sty)*))* ))
        )
    };

    ////////////////////
    // style modifier
    (
        // input
        ( $m:ident ($($mp:expr),*) $($rest:tt)* )
        // output
        ( ($($cur_style:tt)*) $($style_stack:tt)*)
        ( $($ranges:tt)* )
    ) => {

        $crate::__text!(
            ($($rest)*)
            ( ( $($cur_style)* ($m ($($mp),*)) ) $($style_stack)*)
            ($($ranges)*)
        )
    };

    ////////////////////
    // style modifier
    (
        // input
        ( $m:ident $($rest:tt)* )
        // output
        ( ($($cur_style:tt)*) $($style_stack:tt)*)
        ($($ranges:tt)*)
    ) => {

        $crate::__text!(
            ($($rest)*)
            ( ( $($cur_style)* ($m) ) $($style_stack)*)
            ($($ranges)*)
        )
    };

    ////////////////////
    // color modifier (literal ver.)
    (
        // input
        ( # $color:literal $($rest:tt)* )
        // output
        ($($style_stack:tt)*)
        ($($ranges:tt)*)
    ) => {

        $crate::__text!(
            (hexcolor(::std::stringify!($color)) $($rest)*)
            ($($style_stack)*)
            ($($ranges)*)
        )
    };


    ////////////////////
    // color modifier (ident ver. when the color starts with a letter...)
    (
        // input
        ( # $color:ident $($rest:tt)* )
        // output
        ($($style_stack:tt)*)
        ($($ranges:tt)*)
    ) => {

        $crate::__text!(
            (hexcolor(::std::stringify!($color)) $($rest)*)
            ($($style_stack)*)
            ($($ranges)*)
        )
    };


    ////////////////////
    // block start
    (
        // input
        ( { $($inner:tt)* } $($rest:tt)* )
        // output
        ($($style_stack:tt)*)
        ($($ranges:tt)*)
    )
    => {
        $crate::__text!(
            ( $($inner)* @pop $($rest)* )
            (() $($style_stack)*)
            ($($ranges)*)
        )
    };

    /*(@body($runs:ident,$style:ident) $str:literal $($rest:tt)*) => {
        runs.push($crate::text::AttributedRange::owned(format!($str), $style.clone()));
        __text! { @body($runs,$style) $($rest)* };
    };*/
}

/// Macro to create an array of `AttributedRange`s.
///
/// # Example
///
/// ```
///
/// let run = text_run! { size(20.0) "Hello, world!" { b "test" } };
///
#[macro_export]
macro_rules! text {
    ( $($rest:tt)* ) => {
        {
            $crate::__text!(
                ( $($rest)* )
                (())
                ()
            )
        }
    };
}

fn test_text() {
    text!(
        rgb(1,2,3) "Hello, world!"
        { size(42.0) b i "test" i " world" }
        "rest"
    );
}

/// Lines of formatted (shaped and layouted) text.
pub struct FormattedText {
    pub inner: skia_safe::textlayout::Paragraph,
}

impl Default for FormattedText {
    fn default() -> Self {
        let paragraph_style = sk::textlayout::ParagraphStyle::new();
        let font_collection = get_font_collection();
        FormattedText {
            inner: sk::textlayout::ParagraphBuilder::new(&paragraph_style, font_collection).build(),
        }
    }
}

impl FormattedText {
    /// Creates a new formatted text object for the specified text runs (text + associated style).

    // Q: take a slice of AttributedRange? No, because the slice needs to be constructed, maybe unnecessarily.
    // With IntoIterator this works with everything (there are no slices involved)

    pub fn new<'a>(text: impl IntoIterator<Item=AttributedRange<'a>>) -> Self {
        let font_collection = get_font_collection();
        let mut text_style = sk::textlayout::TextStyle::new();
        text_style.set_font_size(16.0 as sk::scalar); // TODO default font size
        let mut paragraph_style = sk::textlayout::ParagraphStyle::new();
        paragraph_style.set_text_style(&text_style);
        let mut builder = sk::textlayout::ParagraphBuilder::new(&paragraph_style, font_collection);

        for run in text.into_iter() {
            let style = run.style.to_skia();
            builder.push_style(&style);
            builder.add_text(&run.str);
            builder.pop();
        }

        Self { inner: builder.build() }
    }

    pub fn from_attributed_str(text: &AttributedStr) -> Self {
        Self::new(text.iter().cloned())
    }

    /// Layouts or relayouts the text under the given width constraint.
    pub fn layout(&mut self, available_width: f64) {
        self.inner.layout(available_width as f32);
    }

    /// Returns bounding rectangles for the specified range of text, specified in byte offsets.
    pub fn get_rects_for_range(&self, range: Range<usize>) -> Vec<Rect> {
        let text_boxes = self.inner.get_rects_for_range(range, RectHeightStyle::Tight, RectWidthStyle::Tight);
        text_boxes.iter().map(|r| Rect::from_skia(r.rect)).collect()
    }
}

/// Text selection.
///
/// Start is the start of the selection, end is the end. The caret is at the end of the selection.
/// Note that we don't necessarily have start <= end: a selection with start > end means that the
/// user started the selection gesture from a later point in the text and then went back
/// (right-to-left in LTR languages). In this case, the cursor will appear at the "beginning"
/// (i.e. left, for LTR) of the selection.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Selection {
    pub start: usize,
    /// The end of the selection, and also the position of the caret. Not necessarily greater than start,
    /// if the selection was made by dragging from right to left.
    pub end: usize,
}

impl Selection {
    pub fn min(&self) -> usize {
        self.start.min(self.end)
    }
    pub fn max(&self) -> usize {
        self.start.max(self.end)
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
    pub fn empty(at: usize) -> Selection {
        Selection { start: at, end: at }
    }
    pub fn byte_range(&self) -> Range<usize> {
        self.min()..self.max()
    }
}

impl Default for Selection {
    fn default() -> Self {
        Selection::empty(0)
    }
}
