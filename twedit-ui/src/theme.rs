//! twedit's visual theme, modeled on the modernized C# EsfEditor look:
//! slate panels, vivid blue accent, flat compact controls.
//!
//! Three mechanisms combine here:
//! 1. `set_requested_theme(Dark)` pins the app to dark regardless of the
//!    Windows setting.
//! 2. `THEME_XAML` is merged into `Application.Resources` after the WinUI
//!    control resources (see `App::theme_resources`). It contains both
//!    lightweight-styling resource overrides AND a full custom
//!    `ControlTemplate` for Button — buttons are drawn entirely by our
//!    template, not the WinUI one.
//! 3. Rust-side constants below for panels the app paints directly.

use windows_reactor::Color;

// Palette (also used from Rust code for direct modifiers).
pub const BASE: Color = Color::rgb(0x18, 0x1B, 0x22); // content background
pub const PANEL: Color = Color::rgb(0x1B, 0x1F, 0x27); // tree panel
pub const HEADER: Color = Color::rgb(0x20, 0x24, 0x2D); // table header strip
pub const BORDER: Color = Color::rgb(0x2E, 0x34, 0x40); // separators
pub const STATUS_BLUE: Color = Color::rgb(0x1E, 0x76, 0xE8); // status bar
pub const TEXT: Color = Color::rgb(0xE4, 0xE6, 0xEB); // primary text
pub const ACCENT: Color = Color::rgb(0x2D, 0x7D, 0xF6); // interactive accent

pub const THEME_XAML: &str = r##"<ResourceDictionary
    xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
    xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml">

    <CornerRadius x:Key="ControlCornerRadius">4</CornerRadius>
    <CornerRadius x:Key="OverlayCornerRadius">6</CornerRadius>

    <!-- Surfaces -->
    <SolidColorBrush x:Key="ApplicationPageBackgroundThemeBrush" Color="#181B22"/>
    <SolidColorBrush x:Key="SolidBackgroundFillColorBaseBrush" Color="#181B22"/>
    <SolidColorBrush x:Key="LayerFillColorDefaultBrush" Color="#1B1F27"/>
    <SolidColorBrush x:Key="CardBackgroundFillColorDefaultBrush" Color="#20242D"/>
    <SolidColorBrush x:Key="SubtleFillColorSecondaryBrush" Color="#232833"/>
    <SolidColorBrush x:Key="DividerStrokeColorDefaultBrush" Color="#2E3440"/>
    <SolidColorBrush x:Key="CardStrokeColorDefaultBrush" Color="#2E3440"/>
    <SolidColorBrush x:Key="SurfaceStrokeColorDefaultBrush" Color="#2E3440"/>

    <!-- Text -->
    <SolidColorBrush x:Key="TextFillColorPrimaryBrush" Color="#E4E6EB"/>
    <SolidColorBrush x:Key="TextFillColorSecondaryBrush" Color="#98A1B0"/>
    <SolidColorBrush x:Key="TextFillColorTertiaryBrush" Color="#6B7382"/>
    <SolidColorBrush x:Key="TextFillColorDisabledBrush" Color="#525A68"/>

    <!-- Accent: vivid blue -->
    <SolidColorBrush x:Key="AccentFillColorDefaultBrush" Color="#2D7DF6"/>
    <SolidColorBrush x:Key="AccentFillColorSecondaryBrush" Color="#4E93FF"/>
    <SolidColorBrush x:Key="AccentFillColorTertiaryBrush" Color="#2361BF"/>
    <SolidColorBrush x:Key="AccentTextFillColorPrimaryBrush" Color="#62A0FF"/>

    <!-- Text boxes: flat dark wells, blue focus -->
    <SolidColorBrush x:Key="TextControlBackground" Color="#12151B"/>
    <SolidColorBrush x:Key="TextControlBackgroundPointerOver" Color="#151920"/>
    <SolidColorBrush x:Key="TextControlBackgroundFocused" Color="#12151B"/>
    <SolidColorBrush x:Key="TextControlBackgroundDisabled" Color="#1B1F27"/>
    <SolidColorBrush x:Key="TextControlForeground" Color="#E4E6EB"/>
    <SolidColorBrush x:Key="TextControlForegroundPointerOver" Color="#E4E6EB"/>
    <SolidColorBrush x:Key="TextControlForegroundFocused" Color="#F2F4F8"/>
    <SolidColorBrush x:Key="TextControlForegroundDisabled" Color="#525A68"/>
    <SolidColorBrush x:Key="TextControlBorderBrush" Color="#333947"/>
    <SolidColorBrush x:Key="TextControlBorderBrushPointerOver" Color="#414A5C"/>
    <SolidColorBrush x:Key="TextControlBorderBrushFocused" Color="#2D7DF6"/>
    <SolidColorBrush x:Key="TextControlBorderBrushDisabled" Color="#272C36"/>
    <SolidColorBrush x:Key="TextControlPlaceholderForeground" Color="#6B7382"/>
    <SolidColorBrush x:Key="TextControlPlaceholderForegroundPointerOver" Color="#6B7382"/>
    <SolidColorBrush x:Key="TextControlPlaceholderForegroundFocused" Color="#6B7382"/>
    <SolidColorBrush x:Key="TextControlSelectionHighlightColor" Color="#2D7DF6"/>

    <!-- Tree view: compact rows, blue selection like the old editor -->
    <x:Double x:Key="TreeViewItemMinHeight">26</x:Double>
    <SolidColorBrush x:Key="TreeViewItemBackgroundPointerOver" Color="#232833"/>
    <SolidColorBrush x:Key="TreeViewItemBackgroundPressed" Color="#1E222A"/>
    <SolidColorBrush x:Key="TreeViewItemBackgroundSelected" Color="#2258A5"/>
    <SolidColorBrush x:Key="TreeViewItemBackgroundSelectedPointerOver" Color="#2A62B5"/>
    <SolidColorBrush x:Key="TreeViewItemBackgroundSelectedPressed" Color="#1D4C90"/>
    <SolidColorBrush x:Key="TreeViewItemForeground" Color="#E4E6EB"/>
    <SolidColorBrush x:Key="TreeViewItemForegroundPointerOver" Color="#F2F4F8"/>
    <SolidColorBrush x:Key="TreeViewItemForegroundSelected" Color="#FFFFFF"/>

    <!-- Toggle switch: blue when on -->
    <SolidColorBrush x:Key="ToggleSwitchFillOn" Color="#2D7DF6"/>
    <SolidColorBrush x:Key="ToggleSwitchFillOnPointerOver" Color="#4E93FF"/>
    <SolidColorBrush x:Key="ToggleSwitchFillOnPressed" Color="#2361BF"/>
    <SolidColorBrush x:Key="ToggleSwitchStrokeOn" Color="#2D7DF6"/>
    <SolidColorBrush x:Key="ToggleSwitchStrokeOnPointerOver" Color="#4E93FF"/>
    <SolidColorBrush x:Key="ToggleSwitchStrokeOnPressed" Color="#2361BF"/>
    <SolidColorBrush x:Key="ToggleSwitchKnobFillOn" Color="#FFFFFF"/>
    <SolidColorBrush x:Key="ToggleSwitchFillOff" Color="#12151B"/>
    <SolidColorBrush x:Key="ToggleSwitchStrokeOff" Color="#414A5C"/>
    <SolidColorBrush x:Key="ToggleSwitchKnobFillOff" Color="#98A1B0"/>

    <!-- List box selection -->
    <SolidColorBrush x:Key="SystemControlHighlightListAccentLowBrush" Color="#662D7DF6"/>
    <SolidColorBrush x:Key="SystemControlHighlightListAccentMediumBrush" Color="#8C2D7DF6"/>
    <SolidColorBrush x:Key="SystemControlHighlightListLowBrush" Color="#232833"/>
    <SolidColorBrush x:Key="SystemControlHighlightListMediumBrush" Color="#2C3340"/>

    <!-- Scrollbars -->
    <SolidColorBrush x:Key="ScrollBarThumbFill" Color="#3A4150"/>
    <SolidColorBrush x:Key="ScrollBarThumbFillPointerOver" Color="#4A5468"/>
    <SolidColorBrush x:Key="ScrollBarThumbFillPressed" Color="#2D7DF6"/>
    <SolidColorBrush x:Key="ScrollBarTrackFill" Color="#181B22"/>

    <!-- Focus rectangles -->
    <SolidColorBrush x:Key="FocusStrokeColorOuterBrush" Color="#2D7DF6"/>
    <SolidColorBrush x:Key="FocusStrokeColorInnerBrush" Color="#181B22"/>

    <!-- Fully custom Button: flat slate chip, blue edge on hover.
         This replaces the WinUI button template outright. -->
    <Style TargetType="Button">
        <Setter Property="Background" Value="#242933"/>
        <Setter Property="Foreground" Value="#E4E6EB"/>
        <Setter Property="BorderBrush" Value="#333947"/>
        <Setter Property="BorderThickness" Value="1"/>
        <Setter Property="Padding" Value="12,5,12,6"/>
        <Setter Property="FontSize" Value="13"/>
        <Setter Property="UseSystemFocusVisuals" Value="True"/>
        <Setter Property="Template">
            <Setter.Value>
                <ControlTemplate TargetType="Button">
                    <Border x:Name="Root"
                            Background="{TemplateBinding Background}"
                            BorderBrush="{TemplateBinding BorderBrush}"
                            BorderThickness="{TemplateBinding BorderThickness}"
                            CornerRadius="4">
                        <VisualStateManager.VisualStateGroups>
                            <VisualStateGroup x:Name="CommonStates">
                                <VisualState x:Name="Normal"/>
                                <VisualState x:Name="PointerOver">
                                    <VisualState.Setters>
                                        <Setter Target="Root.Background" Value="#2C3340"/>
                                        <Setter Target="Root.BorderBrush" Value="#3E70C4"/>
                                    </VisualState.Setters>
                                </VisualState>
                                <VisualState x:Name="Pressed">
                                    <VisualState.Setters>
                                        <Setter Target="Root.Background" Value="#1B1F27"/>
                                        <Setter Target="Root.BorderBrush" Value="#2D7DF6"/>
                                    </VisualState.Setters>
                                </VisualState>
                                <VisualState x:Name="Disabled">
                                    <VisualState.Setters>
                                        <Setter Target="Root.Background" Value="#1E222A"/>
                                        <Setter Target="Root.BorderBrush" Value="#272C36"/>
                                        <Setter Target="Presenter.Foreground" Value="#525A68"/>
                                    </VisualState.Setters>
                                </VisualState>
                            </VisualStateGroup>
                        </VisualStateManager.VisualStateGroups>
                        <ContentPresenter x:Name="Presenter"
                                          Content="{TemplateBinding Content}"
                                          ContentTemplate="{TemplateBinding ContentTemplate}"
                                          ContentTransitions="{TemplateBinding ContentTransitions}"
                                          Padding="{TemplateBinding Padding}"
                                          HorizontalContentAlignment="Center"
                                          VerticalContentAlignment="Center"
                                          AutomationProperties.AccessibilityView="Raw"/>
                    </Border>
                </ControlTemplate>
            </Setter.Value>
        </Setter>
    </Style>
</ResourceDictionary>"##;
