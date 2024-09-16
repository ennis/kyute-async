use std::any::TypeId;
use std::hash::Hash;

use paste::paste;

use crate::Color;
use crate::drawing::BoxShadow;
use crate::layout::{Alignment, LengthOrPercentage, Sizing};
use crate::layout::flex::Axis;


trait IntoStyleValue {
    fn into_style_value(self) -> StyleValue;
    fn from_style_value(value: StyleValue) -> Self;
}

macro_rules! impl_style_values {
    (
        $($ty:ty, $variant:ident;)*
    ) => {
        #[derive(Clone)]
        enum StyleValue {
            $($variant($ty),)*
        }

        $(impl IntoStyleValue for $ty {
            fn into_style_value(self) -> StyleValue {
                StyleValue::$variant(self)
            }
            fn from_style_value(value: StyleValue) -> Self {
                match value {
                    StyleValue::$variant(v) => v,
                    _ => panic!("invalid style value"),
                }
            }
        })*
    };
}

impl_style_values!(
    LengthOrPercentage, LengthOrPercentage;
    Alignment, Alignment;
    crate::layout::flex::MainAxisAlignment, MainAxisAlignment;
    crate::layout::flex::CrossAxisAlignment, CrossAxisAlignment;
    Axis, Axis;
    Color, Color;
    f64, Float;
    u32, U32;
    bool, Bool;
    String, String;
    Style, Style;
    Sizing, Sizing;
    Vec<BoxShadow>, BoxShadows;
    CustomFontAxisValue, CustomFontAxisValue;
);

pub trait StyleProperty: 'static {
    type Value: IntoStyleValue;
}

macro_rules! style_properties {
    (
        $($name:ident: $ty:ty;)*
    ) => {
        paste::paste! {
            $(
                pub struct $name;
                impl StyleProperty for $name {
                    type Value = $ty;
                }
            )*

            pub trait StyleExt {
                $(
                    fn [<$name:snake>](self, value: <$name as StyleProperty>::Value) -> Self;
                )*
            }

            impl StyleExt for Style {
                $(
                    fn [<$name:snake>](mut self, value: <$name as StyleProperty>::Value) -> Self {
                        self.set($name, value);
                        self
                    }
                )*
            }
        }
    };
}

style_properties! {
    PaddingLeft: LengthOrPercentage;
    PaddingRight: LengthOrPercentage;
    PaddingTop: LengthOrPercentage;
    PaddingBottom: LengthOrPercentage;
    HorizontalAlign: Alignment;
    VerticalAlign: Alignment;
    Baseline: LengthOrPercentage;
    Width: Sizing;
    Height: Sizing;
    Direction: Axis;
    CrossAxisAlignment: crate::layout::flex::CrossAxisAlignment;
    MainAxisAlignment: crate::layout::flex::MainAxisAlignment;
    FlexFactor: f64;
    BorderLeft: LengthOrPercentage;
    BorderRight: LengthOrPercentage;
    BorderTop: LengthOrPercentage;
    BorderBottom: LengthOrPercentage;
    BorderColor: Color;
    BorderRadius: f64;
    BackgroundColor: Color;
    BoxShadows: Vec<BoxShadow>;
    MinWidth: LengthOrPercentage;
    MinHeight: LengthOrPercentage;
    MaxWidth: LengthOrPercentage;
    MaxHeight: LengthOrPercentage;


    // Pseudo states
    Active: Style;
    Hover: Style;
    Focus: Style;
}

pub(crate) use style_properties;
use crate::text::CustomFontAxisValue;

#[derive(Clone, Default)]
pub struct Style {
    values: imbl::OrdMap<TypeId, StyleValue>,
}

impl Style {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn set<P: StyleProperty>(&mut self, _p: P, value: P::Value) {
        self.values.insert(TypeId::of::<P>(), value.into_style_value());
    }

    pub fn get<P: StyleProperty>(&self, _p: P) -> Option<P::Value> {
        self.values
            .get(&TypeId::of::<P>())
            .map(|v| P::Value::from_style_value(v.clone()))
    }

    pub fn get_or_default<P: StyleProperty>(&self, _p: P) -> P::Value
    where
        P::Value: Default,
    {
        self.get(_p).unwrap_or_default()
    }

    pub fn over(self, other: Self) -> Self {
        Style {
            values: self.values.union(other.values),
        }
    }
}

impl PartialEq for Style {
    fn eq(&self, other: &Self) -> bool {
        self.values.ptr_eq(&other.values)
    }
}

impl Eq for Style {}


/*
pub struct RuleSetInner {
    pub parent: Option<Rc<RuleSetInner>>,
    pub properties: Vec<StyleProperty>,
}

#[derive(Clone)]
pub struct RuleSet(Rc<RuleSetInner>);*/

// Stylesheet example usage:
// let mut stylesheet = Stylesheet::new();
// stylesheet.set_padding_left(LengthOrPercentage::Length(10.0));
// stylesheet.set_padding_right(LengthOrPercentage::Length(10.0));
// Do we need classes?

// One style per frame.
// Issue: need attached properties on sub-elements for things like flex-factor, etc.
//  -> would not need that if styling info was on every element, and flex-factor was in the style info
//  -> on the other hand, removing attached properties would make it less flexible for custom layouts outside the framework (docking?)
//  -> attached properties could be put in the style itself
//
// Issue: inheritance (text-color, font-family, etc.)
// -> do we need a cascade?

// Not sure that the styling system should be integrated so tightly with the layout system.
// However attached properties are annoying to work with.
// -> the root cause is that the containers do not own their children
// -> keep attached properties for now

// Decision:
// - keep attached properties for now
// - styles only apply to frames, so no text properties in the style
// - no cascade
// - no classes
// - no inheritance

// Q: should frames handle the layout of their children?
// A: yes => frames should have a flex layout by default, but replaceable with custom layout (via a trait)
//   benefits: one less element in the hierarchy: Frame { direction: Vertical, children ... } instead of Frame { Column { children ... } }
//   generally, single-element containers should be reduced to a minimum

// Q: styling of text and inheritance: where do text elements get their font family and size from?
// Options:
// - set explicitly for each element
// - inherited from parent frame
//
// Q: Should text elements be styled with a style object?
// -> what would the element do with decorated box properties (like background, border, padding, etc.)? Should it ignore them?
// A: text elements shouldn't have a style object; propagate style info in other ways
//

// Alternative:
// Style = generic container, for an unbounded set of props
// - a style can have sub-styles for element states (active, hover, etc.)
//
// Style resolution:
// - styles are specific to element types
