use super::SupportedLanguage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParserStrategy {
    Heuristic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LanguageCapability {
    pub strategy: ParserStrategy,
    pub supports_symbol_extraction: bool,
    pub supports_import_extraction: bool,
    pub supports_call_heritage_extraction: bool,
}

pub(super) fn language_capability(language: SupportedLanguage) -> LanguageCapability {
    match language {
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => LanguageCapability {
            strategy: ParserStrategy::Heuristic,
            supports_symbol_extraction: true,
            supports_import_extraction: true,
            supports_call_heritage_extraction: true,
        },
        SupportedLanguage::CSharp | SupportedLanguage::Go | SupportedLanguage::PHP => {
            LanguageCapability {
                strategy: ParserStrategy::Heuristic,
                supports_symbol_extraction: true,
                supports_import_extraction: true,
                supports_call_heritage_extraction: false,
            }
        }
        SupportedLanguage::Rust
        | SupportedLanguage::Python
        | SupportedLanguage::Java
        | SupportedLanguage::C
        | SupportedLanguage::Cpp
        | SupportedLanguage::Kotlin
        | SupportedLanguage::Swift => LanguageCapability {
            strategy: ParserStrategy::Heuristic,
            supports_symbol_extraction: true,
            supports_import_extraction: false,
            supports_call_heritage_extraction: false,
        },
    }
}

pub(super) fn supports_symbol_extraction(language: SupportedLanguage) -> bool {
    language_capability(language).supports_symbol_extraction
}

pub(super) fn supports_import_extraction(language: SupportedLanguage) -> bool {
    language_capability(language).supports_import_extraction
}

pub(super) fn supports_call_heritage_extraction(language: SupportedLanguage) -> bool {
    language_capability(language).supports_call_heritage_extraction
}

// Kept for compatibility with existing call sites.
#[allow(dead_code)]
pub(super) fn supports_relation_extraction(language: SupportedLanguage) -> bool {
    let capability = language_capability(language);
    capability.supports_import_extraction || capability.supports_call_heritage_extraction
}

#[cfg(test)]
mod tests {
    use super::{SupportedLanguage, supports_call_heritage_extraction, supports_import_extraction};

    #[test]
    fn csharp_go_php_are_import_only_for_relations() {
        for language in [
            SupportedLanguage::CSharp,
            SupportedLanguage::Go,
            SupportedLanguage::PHP,
        ] {
            assert!(supports_import_extraction(language));
            assert!(!supports_call_heritage_extraction(language));
        }
    }
}
