//! twedit's visual identity: an "imperial ledger" look — near-black umber
//! surfaces, parchment text, antique-gold accents, hairline bronze borders.
//! Deliberately unlike stock WinUI: no mica/acrylic grays, no system blue.
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
//!
//! Bad XAML here fails at app START (not compile time) — smoke-launch after
//! any edit to `THEME_XAML`.

use windows_reactor::Color;

// Palette (also used from Rust code for direct modifiers).
pub const BASE: Color = Color::rgb(0x14, 0x12, 0x0D); // app background, warm near-black
pub const PANEL: Color = Color::rgb(0x1A, 0x17, 0x11); // card panels
pub const HEADER: Color = Color::rgb(0x22, 0x1E, 0x15); // table header / status strips
pub const BORDER: Color = Color::rgb(0x3A, 0x33, 0x24); // hairline bronze separators
pub const TEXT: Color = Color::rgb(0xEA, 0xE3, 0xD0); // parchment primary text
pub const TEXT_DIM: Color = Color::rgb(0xA8, 0x9F, 0x88); // secondary text
pub const ACCENT: Color = Color::rgb(0xC9, 0xA2, 0x3F); // antique gold
pub const ACCENT_BRIGHT: Color = Color::rgb(0xE3, 0xBE, 0x5C); // gold, hover
pub const CRIMSON: Color = Color::rgb(0xA8, 0x3A, 0x32); // war/destructive accent
pub const STATUS_TICK: Color = ACCENT; // status bar accent tick
pub const ROW_ALT: Color = Color::rgb(0x17, 0x14, 0x0F); // zebra stripe rows
pub const SWATCH_BORDER: Color = Color::rgb(0x4A, 0x42, 0x30); // faction colour chips

pub const THEME_XAML: &str = r##"<ResourceDictionary
    xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
    xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml">

    <CornerRadius x:Key="ControlCornerRadius">3</CornerRadius>
    <CornerRadius x:Key="OverlayCornerRadius">5</CornerRadius>

    <!-- Surfaces: warm near-black umber, never system gray -->
    <SolidColorBrush x:Key="ApplicationPageBackgroundThemeBrush" Color="#14120D"/>
    <SolidColorBrush x:Key="SolidBackgroundFillColorBaseBrush" Color="#14120D"/>
    <SolidColorBrush x:Key="SolidBackgroundFillColorSecondaryBrush" Color="#1A1711"/>
    <SolidColorBrush x:Key="LayerFillColorDefaultBrush" Color="#1A1711"/>
    <SolidColorBrush x:Key="CardBackgroundFillColorDefaultBrush" Color="#221E15"/>
    <SolidColorBrush x:Key="SubtleFillColorSecondaryBrush" Color="#2A2519"/>
    <SolidColorBrush x:Key="SubtleFillColorTertiaryBrush" Color="#241F16"/>
    <SolidColorBrush x:Key="DividerStrokeColorDefaultBrush" Color="#3A3324"/>
    <SolidColorBrush x:Key="CardStrokeColorDefaultBrush" Color="#3A3324"/>
    <SolidColorBrush x:Key="SurfaceStrokeColorDefaultBrush" Color="#3A3324"/>
    <SolidColorBrush x:Key="AcrylicBackgroundFillColorDefaultBrush" Color="#1A1711"/>
    <SolidColorBrush x:Key="AcrylicInAppFillColorDefaultBrush" Color="#1A1711"/>

    <!-- Text: parchment -->
    <SolidColorBrush x:Key="TextFillColorPrimaryBrush" Color="#EAE3D0"/>
    <SolidColorBrush x:Key="TextFillColorSecondaryBrush" Color="#A89F88"/>
    <SolidColorBrush x:Key="TextFillColorTertiaryBrush" Color="#736C59"/>
    <SolidColorBrush x:Key="TextFillColorDisabledBrush" Color="#57503F"/>

    <!-- Accent: antique gold -->
    <SolidColorBrush x:Key="AccentFillColorDefaultBrush" Color="#C9A23F"/>
    <SolidColorBrush x:Key="AccentFillColorSecondaryBrush" Color="#E3BE5C"/>
    <SolidColorBrush x:Key="AccentFillColorTertiaryBrush" Color="#9A7B26"/>
    <SolidColorBrush x:Key="AccentTextFillColorPrimaryBrush" Color="#DDB955"/>
    <SolidColorBrush x:Key="AccentTextFillColorSecondaryBrush" Color="#E3BE5C"/>
    <SolidColorBrush x:Key="AccentTextFillColorTertiaryBrush" Color="#C9A23F"/>
    <SolidColorBrush x:Key="AccentFillColorSelectedTextBackgroundBrush" Color="#C9A23F"/>

    <!-- Text boxes: flat dark wells, gold focus -->
    <SolidColorBrush x:Key="TextControlBackground" Color="#0F0D09"/>
    <SolidColorBrush x:Key="TextControlBackgroundPointerOver" Color="#13100B"/>
    <SolidColorBrush x:Key="TextControlBackgroundFocused" Color="#0F0D09"/>
    <SolidColorBrush x:Key="TextControlBackgroundDisabled" Color="#1A1711"/>
    <SolidColorBrush x:Key="TextControlForeground" Color="#EAE3D0"/>
    <SolidColorBrush x:Key="TextControlForegroundPointerOver" Color="#EAE3D0"/>
    <SolidColorBrush x:Key="TextControlForegroundFocused" Color="#F5EFDD"/>
    <SolidColorBrush x:Key="TextControlForegroundDisabled" Color="#57503F"/>
    <SolidColorBrush x:Key="TextControlBorderBrush" Color="#3F3826"/>
    <SolidColorBrush x:Key="TextControlBorderBrushPointerOver" Color="#57503F"/>
    <SolidColorBrush x:Key="TextControlBorderBrushFocused" Color="#C9A23F"/>
    <SolidColorBrush x:Key="TextControlBorderBrushDisabled" Color="#2A2519"/>
    <SolidColorBrush x:Key="TextControlPlaceholderForeground" Color="#736C59"/>
    <SolidColorBrush x:Key="TextControlPlaceholderForegroundPointerOver" Color="#736C59"/>
    <SolidColorBrush x:Key="TextControlPlaceholderForegroundFocused" Color="#736C59"/>
    <SolidColorBrush x:Key="TextControlSelectionHighlightColor" Color="#8A6E1F"/>

    <!-- Tree view: compact rows, gold-tinted selection -->
    <x:Double x:Key="TreeViewItemMinHeight">26</x:Double>
    <SolidColorBrush x:Key="TreeViewItemBackgroundPointerOver" Color="#2A2519"/>
    <SolidColorBrush x:Key="TreeViewItemBackgroundPressed" Color="#221E15"/>
    <SolidColorBrush x:Key="TreeViewItemBackgroundSelected" Color="#4A3B18"/>
    <SolidColorBrush x:Key="TreeViewItemBackgroundSelectedPointerOver" Color="#57451D"/>
    <SolidColorBrush x:Key="TreeViewItemBackgroundSelectedPressed" Color="#3D3113"/>
    <SolidColorBrush x:Key="TreeViewItemForeground" Color="#EAE3D0"/>
    <SolidColorBrush x:Key="TreeViewItemForegroundPointerOver" Color="#F5EFDD"/>
    <SolidColorBrush x:Key="TreeViewItemForegroundSelected" Color="#F5EFDD"/>

    <!-- Toggle switch: gold when on -->
    <SolidColorBrush x:Key="ToggleSwitchFillOn" Color="#C9A23F"/>
    <SolidColorBrush x:Key="ToggleSwitchFillOnPointerOver" Color="#E3BE5C"/>
    <SolidColorBrush x:Key="ToggleSwitchFillOnPressed" Color="#9A7B26"/>
    <SolidColorBrush x:Key="ToggleSwitchStrokeOn" Color="#C9A23F"/>
    <SolidColorBrush x:Key="ToggleSwitchStrokeOnPointerOver" Color="#E3BE5C"/>
    <SolidColorBrush x:Key="ToggleSwitchStrokeOnPressed" Color="#9A7B26"/>
    <SolidColorBrush x:Key="ToggleSwitchKnobFillOn" Color="#14120D"/>
    <SolidColorBrush x:Key="ToggleSwitchFillOff" Color="#0F0D09"/>
    <SolidColorBrush x:Key="ToggleSwitchStrokeOff" Color="#57503F"/>
    <SolidColorBrush x:Key="ToggleSwitchKnobFillOff" Color="#A89F88"/>

    <!-- List box selection -->
    <SolidColorBrush x:Key="SystemControlHighlightListAccentLowBrush" Color="#66C9A23F"/>
    <SolidColorBrush x:Key="SystemControlHighlightListAccentMediumBrush" Color="#8CC9A23F"/>
    <SolidColorBrush x:Key="SystemControlHighlightListLowBrush" Color="#2A2519"/>
    <SolidColorBrush x:Key="SystemControlHighlightListMediumBrush" Color="#332C1D"/>

    <!-- Scrollbars -->
    <SolidColorBrush x:Key="ScrollBarThumbFill" Color="#4A4231"/>
    <SolidColorBrush x:Key="ScrollBarThumbFillPointerOver" Color="#5C5340"/>
    <SolidColorBrush x:Key="ScrollBarThumbFillPressed" Color="#C9A23F"/>
    <SolidColorBrush x:Key="ScrollBarTrackFill" Color="#14120D"/>

    <!-- Focus rectangles -->
    <SolidColorBrush x:Key="FocusStrokeColorOuterBrush" Color="#C9A23F"/>
    <SolidColorBrush x:Key="FocusStrokeColorInnerBrush" Color="#14120D"/>

    <!-- Menu flyouts / tooltips -->
    <SolidColorBrush x:Key="MenuFlyoutPresenterBackground" Color="#1E1A12"/>
    <SolidColorBrush x:Key="MenuFlyoutPresenterBorderBrush" Color="#3A3324"/>
    <SolidColorBrush x:Key="MenuFlyoutItemBackgroundPointerOver" Color="#2E2818"/>
    <SolidColorBrush x:Key="MenuFlyoutItemBackgroundPressed" Color="#241F16"/>
    <SolidColorBrush x:Key="ToolTipBackground" Color="#1E1A12"/>
    <SolidColorBrush x:Key="ToolTipForeground" Color="#EAE3D0"/>
    <SolidColorBrush x:Key="ToolTipBorderBrush" Color="#3A3324"/>

    <!-- Breadcrumb bar -->
    <SolidColorBrush x:Key="BreadcrumbBarNormalForegroundBrush" Color="#A89F88"/>
    <SolidColorBrush x:Key="BreadcrumbBarHoverForegroundBrush" Color="#E3BE5C"/>
    <SolidColorBrush x:Key="BreadcrumbBarPressedForegroundBrush" Color="#C9A23F"/>
    <SolidColorBrush x:Key="BreadcrumbBarCurrentNormalForegroundBrush" Color="#EAE3D0"/>

    <!-- Fully custom Button: flat umber chip, gold edge on hover.
         This replaces the WinUI button template outright. -->
    <Style TargetType="Button">
        <Setter Property="Background" Value="#262115"/>
        <Setter Property="Foreground" Value="#EAE3D0"/>
        <Setter Property="BorderBrush" Value="#3F3826"/>
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
                            CornerRadius="3">
                        <VisualStateManager.VisualStateGroups>
                            <VisualStateGroup x:Name="CommonStates">
                                <VisualState x:Name="Normal"/>
                                <VisualState x:Name="PointerOver">
                                    <VisualState.Setters>
                                        <Setter Target="Root.Background" Value="#312A18"/>
                                        <Setter Target="Root.BorderBrush" Value="#B08D3B"/>
                                    </VisualState.Setters>
                                </VisualState>
                                <VisualState x:Name="Pressed">
                                    <VisualState.Setters>
                                        <Setter Target="Root.Background" Value="#1A1711"/>
                                        <Setter Target="Root.BorderBrush" Value="#C9A23F"/>
                                    </VisualState.Setters>
                                </VisualState>
                                <VisualState x:Name="Disabled">
                                    <VisualState.Setters>
                                        <Setter Target="Root.Background" Value="#1E1A12"/>
                                        <Setter Target="Root.BorderBrush" Value="#2A2519"/>
                                        <Setter Target="Presenter.Foreground" Value="#57503F"/>
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

    <!-- Fully custom TextBox: a flat umber well with a gold underline on
         focus (no WinUI double-border / reveal edge). Requires the named
         parts ContentElement (ScrollViewer) + PlaceholderTextContentPresenter
         the control toggles by name. -->
    <Style TargetType="TextBox">
        <Setter Property="Background" Value="#0F0D09"/>
        <Setter Property="Foreground" Value="#EAE3D0"/>
        <Setter Property="BorderBrush" Value="#3F3826"/>
        <Setter Property="BorderThickness" Value="1"/>
        <Setter Property="Padding" Value="8,5,8,6"/>
        <Setter Property="FontSize" Value="13"/>
        <Setter Property="Template">
            <Setter.Value>
                <ControlTemplate TargetType="TextBox">
                    <Grid>
                        <Grid.RowDefinitions>
                            <RowDefinition Height="Auto"/>
                            <RowDefinition Height="*"/>
                        </Grid.RowDefinitions>
                        <VisualStateManager.VisualStateGroups>
                            <VisualStateGroup x:Name="CommonStates">
                                <VisualState x:Name="Normal"/>
                                <VisualState x:Name="PointerOver">
                                    <VisualState.Setters>
                                        <Setter Target="BorderElement.BorderBrush" Value="#57503F"/>
                                    </VisualState.Setters>
                                </VisualState>
                                <VisualState x:Name="Focused">
                                    <VisualState.Setters>
                                        <Setter Target="BorderElement.BorderBrush" Value="#C9A23F"/>
                                        <Setter Target="BorderElement.BorderThickness" Value="1,1,1,2"/>
                                    </VisualState.Setters>
                                </VisualState>
                                <VisualState x:Name="Disabled">
                                    <VisualState.Setters>
                                        <Setter Target="BorderElement.Background" Value="#1A1711"/>
                                        <Setter Target="ContentElement.Foreground" Value="#57503F"/>
                                    </VisualState.Setters>
                                </VisualState>
                            </VisualStateGroup>
                        </VisualStateManager.VisualStateGroups>
                        <Border x:Name="BorderElement" Grid.Row="1"
                                Background="{TemplateBinding Background}"
                                BorderBrush="{TemplateBinding BorderBrush}"
                                BorderThickness="{TemplateBinding BorderThickness}"
                                CornerRadius="3"/>
                        <TextBlock x:Name="PlaceholderTextContentPresenter" Grid.Row="1"
                                Text="{TemplateBinding PlaceholderText}"
                                Foreground="#736C59"
                                Margin="{TemplateBinding BorderThickness}"
                                Padding="{TemplateBinding Padding}"
                                IsHitTestVisible="False"
                                TextWrapping="NoWrap"/>
                        <ScrollViewer x:Name="ContentElement" Grid.Row="1"
                                Margin="{TemplateBinding BorderThickness}"
                                Padding="{TemplateBinding Padding}"
                                HorizontalScrollBarVisibility="Hidden"
                                VerticalScrollBarVisibility="Hidden"
                                IsTabStop="False"
                                AutomationProperties.AccessibilityView="Raw"/>
                    </Grid>
                </ControlTemplate>
            </Setter.Value>
        </Setter>
    </Style>
</ResourceDictionary>"##;
