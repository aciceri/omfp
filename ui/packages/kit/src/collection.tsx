import { type ReactNode, useEffect, useRef } from "react";
import {
  Button as AriaButton,
  ListBox as AriaListBox,
  ListBoxItem as AriaListBoxItem,
  type ListBoxItemProps as AriaListBoxItemProps,
  type ListBoxProps as AriaListBoxProps,
  Menu as AriaMenu,
  MenuItem as AriaMenuItem,
  type MenuItemProps as AriaMenuItemProps,
  type MenuProps as AriaMenuProps,
  MenuSection as AriaMenuSection,
  type MenuSectionProps as AriaMenuSectionProps,
  Select as AriaSelect,
  type SelectProps as AriaSelectProps,
  Separator as AriaSeparator,
  type SeparatorProps as AriaSeparatorProps,
  Collection,
  Header,
  type Key,
  Label,
  SelectValue,
} from "react-aria-components";
import { cx, withClass } from "./class.ts";
import { Popover } from "./dialog.tsx";
import { Icon } from "./icons.tsx";
import { Kbd } from "./kbd.tsx";

export { type Key, MenuTrigger, SubmenuTrigger } from "react-aria-components";

export type ListBoxProps<T extends object> = AriaListBoxProps<T>;

/** A keyboard-navigable list of selectable options (arrow keys, typeahead, `aria-selected`). */
export function ListBox<T extends object>({ className, ...props }: ListBoxProps<T>) {
  return <AriaListBox {...props} className={withClass("omfp-listbox", className)} />;
}

export type ListBoxItemProps<T extends object> = AriaListBoxItemProps<T>;

export function ListBoxItem<T extends object>({ className, ...props }: ListBoxItemProps<T>) {
  return <AriaListBoxItem {...props} className={withClass("omfp-listbox__item", className)} />;
}

export type MenuProps<T extends object> = AriaMenuProps<T>;

/** The items of a menu, in order (radio and checkbox items too), without the disabled ones. */
const MENU_ITEMS = '[role^="menuitem"]:not([aria-disabled="true"])';

/**
 * A menu of actions; put it in a `Popover` inside a `MenuTrigger`. Like a native menu, an arrow key while no item
 * shows the focus (opened with the mouse) lands on the first item (ArrowDown) or the last (ArrowUp): React Aria
 * would move on from the item the pointer last crossed, which it keeps focused unseen (UI-288).
 */
export function Menu<T extends object>({ className, ...props }: MenuProps<T>) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const menu = ref.current;
    if (menu === null) {
      return;
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") {
        return;
      }
      const item = event.target instanceof Element ? event.target.closest('[role^="menuitem"]') : null;
      if (item === null || item.hasAttribute("data-focus-visible") || item.hasAttribute("data-hovered")) {
        return;
      }
      const items = [...menu.querySelectorAll<HTMLElement>(MENU_ITEMS)];
      const target = event.key === "ArrowDown" ? items[0] : items.at(-1);
      if (target === undefined) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      target.focus();
    };
    menu.addEventListener("keydown", onKeyDown, { capture: true });
    return () => menu.removeEventListener("keydown", onKeyDown, { capture: true });
  }, []);
  return <AriaMenu {...props} ref={ref} className={withClass("omfp-menu", className)} />;
}

export interface MenuItemProps<T extends object> extends AriaMenuItemProps<T> {
  /** A leading icon (an `Icon`). */
  icon?: ReactNode;
  /** A trailing key chip (UI-148). */
  kbd?: string | undefined;
  /** A destructive action: error text and icon (UI-155). */
  danger?: boolean;
}

/** A menu row (UI-155): icon, label, key chip. */
export function MenuItem<T extends object>({
  icon,
  kbd,
  danger = false,
  className,
  children,
  textValue,
  ...props
}: MenuItemProps<T>) {
  const decorated = icon !== undefined || kbd !== undefined;
  const text = textValue ?? (typeof children === "string" ? children : undefined);
  return (
    <AriaMenuItem
      {...props}
      {...(text === undefined ? {} : { textValue: text })}
      className={withClass("omfp-menu__item", className)}
      data-danger={danger || undefined}
    >
      {decorated && typeof children !== "function" ? (
        <>
          {icon}
          <span className="omfp-menu__label">{children}</span>
          {kbd !== undefined && <Kbd aria-hidden="true">{kbd}</Kbd>}
        </>
      ) : (
        children
      )}
    </AriaMenuItem>
  );
}

export interface MenuSectionProps<T extends object> extends AriaMenuSectionProps<T> {
  /** Visible section heading. */
  title?: ReactNode;
}

export function MenuSection<T extends object>({ title, children, items, ...props }: MenuSectionProps<T>) {
  return (
    <AriaMenuSection {...props}>
      {title !== undefined && <Header className="omfp-menu__header">{title}</Header>}
      {typeof children === "function" ? (
        <Collection {...(items === undefined ? {} : { items })}>{children}</Collection>
      ) : (
        children
      )}
    </AriaMenuSection>
  );
}

export type SeparatorProps = AriaSeparatorProps;

/** A divider between menu sections or toolbar groups. */
export function Separator({ className, ...props }: SeparatorProps) {
  return <AriaSeparator {...props} className={cx("omfp-separator", className)} />;
}

export interface SelectItem {
  id: Key;
  label: string;
  /** Metadata at the option's end (fs-2xs). */
  meta?: string;
  isDisabled?: boolean;
}

export interface SelectProps
  extends Omit<AriaSelectProps<SelectItem>, "children" | "value" | "defaultValue" | "onChange" | "selectionMode"> {
  label: string;
  hideLabel?: boolean;
  items: readonly SelectItem[];
  selectedKey: Key | null;
  onSelectionChange: (key: Key | null) => void;
  /** A leading icon (an `Icon`) in the control. */
  icon?: ReactNode;
  /** A visible word before the value, inside the control (e.g. `thinking`). */
  prefix?: string;
  /** Only the icon shows in the control (the value is its tooltip and accessible text); the list opens as usual. */
  iconOnly?: boolean;
}

/**
 * A labelled single-choice picker (UI-152): a small control showing an optional prefix, the value and a chevron; a
 * listbox in a popover with a check on the selected option.
 */
export function Select({
  icon,
  prefix,
  iconOnly = false,
  label,
  hideLabel = false,
  items,
  selectedKey,
  onSelectionChange,
  className,
  ...props
}: SelectProps) {
  const selected = items.find((item) => item.id === selectedKey)?.label;
  return (
    <AriaSelect<SelectItem>
      {...props}
      value={selectedKey}
      onChange={onSelectionChange}
      className={withClass("omfp-field omfp-select", className)}
      data-icon-only={iconOnly || undefined}
    >
      <Label className={cx("omfp-field__label", hideLabel && "omfp-visually-hidden")}>{label}</Label>
      <AriaButton className="omfp-select__button">
        {iconOnly ? <span title={selected === undefined ? label : `${label}: ${selected}`}>{icon}</span> : icon}
        {prefix !== undefined && <span className="omfp-select__prefix">{prefix}</span>}
        <SelectValue<SelectItem> className={cx("omfp-select__value", iconOnly && "omfp-visually-hidden")}>
          {({ isPlaceholder, selectedText, defaultChildren }) => (isPlaceholder ? defaultChildren : selectedText)}
        </SelectValue>
        {!iconOnly && <Icon name="chevron-down" className="omfp-select__chevron" />}
      </AriaButton>
      <Popover className="omfp-select__popover">
        <ListBox<SelectItem> items={items}>
          {(item) => (
            <SelectOption
              id={item.id}
              textValue={item.label}
              {...(item.isDisabled === undefined ? {} : { isDisabled: item.isDisabled })}
              data-testid="select-option"
              data-key={String(item.id)}
              meta={item.meta}
            >
              {item.label}
            </SelectOption>
          )}
        </ListBox>
      </Popover>
    </AriaSelect>
  );
}

export interface SelectOptionProps<T extends object> extends Omit<ListBoxItemProps<T>, "children"> {
  /** Metadata at the end (fs-2xs, on-surface-variant). */
  meta?: string | undefined;
  children: ReactNode;
}

/** A listbox option with a check column (primary check when selected), label and metadata (UI-152). */
export function SelectOption<T extends object>({ meta, children, className, ...props }: SelectOptionProps<T>) {
  return (
    <AriaListBoxItem {...props} className={withClass("omfp-listbox__item", className)}>
      {({ isSelected }) => (
        <>
          <span className="omfp-option__check">{isSelected && <Icon name="check" />}</span>
          <span className="omfp-option__label">{children}</span>
          {meta !== undefined && <span className="omfp-option__meta">{meta}</span>}
        </>
      )}
    </AriaListBoxItem>
  );
}
