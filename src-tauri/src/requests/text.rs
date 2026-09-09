//! What a queued request says when it reaches an agent (§7.7, OPEN-05, FM-31).
//!
//! Two sentences, in the two languages of APP-02, written where every other user-visible
//! text of this app is written — `src/locales/{en,it}.json` — and never inline here (T-028).
//! The clipboard renders them in the user's language (OPEN-05); the Stop hook renders them
//! in English, because §7.5 fixes the language of a block reason and the reader there is the
//! agent, not the person.
//!
//! **Three things are invariant across both languages**, and the tests below pin all three:
//! the `hf_` id, the tool name `handoff_to_user`, and the parameter name (`request_id=` or
//! `handoff_id=`). They are what the agent acts on; translating them would produce a
//! sentence a person understands and an agent cannot follow.

use crate::i18n::{text, Language};

use super::queue::UserRequest;

/// The catalogue key of the sentence that asks for a spec (OPEN-05).
const REQUEST_KEY: &str = "request.clipboard";

/// The catalogue key of the sentence that asks an agent to come back (FM-31).
const RESUME_KEY: &str = "request.resume";

/// The §7.7 sentence: the user opened a request, produce the spec and quote its id.
#[must_use]
pub fn render_request_text(language: Language, id: &str, request_text: &str) -> String {
    // `{id}` first and `{text}` last: what the user typed is substituted into the sentence
    // and never scanned again, so a request whose own words contain `{id}` stays their
    // words.
    text(language, REQUEST_KEY)
        .replace("{id}", id)
        .replace("{text}", request_text)
}

/// The FM-31 sentence: the user picked a handoff up in the overlay and is waiting.
#[must_use]
pub fn render_resume_text(language: Language, handoff_id: &str) -> String {
    text(language, RESUME_KEY).replace("{id}", handoff_id)
}

/// Whichever of the two `request` is, rendered.
///
/// A queue entry that names a handoff asks an agent to come back to it; one that names none
/// asks for a spec (`log::user_requests`, `migrations/0002`).
#[must_use]
pub fn render_for(language: Language, request: &UserRequest) -> String {
    match request.about_handoff_id.as_deref() {
        Some(handoff_id) => render_resume_text(language, handoff_id),
        None => render_request_text(language, &request.id, &request.text),
    }
}

/// What a resume request stores in its `text` column.
///
/// Short, English and language-free: the sentence an agent reads is rendered from
/// `about_handoff_id` at the moment it is delivered, so that a user who changes the UI
/// language does not find yesterday's queue in yesterday's language.
#[must_use]
pub fn resume_queue_text(handoff_id: &str) -> String {
    format!("Resume handoff {handoff_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::testing::request;

    const ID: &str = "hf_7k3m9p2q4r";
    const WHAT: &str = "I'm about to create the API key on Stripe";

    #[test]
    fn the_english_sentence_is_the_one_of_77_to_the_byte() {
        // §7.7 and OPEN-05 print it; §1.3 makes a text of the design normative.
        assert_eq!(
            render_request_text(Language::En, ID, WHAT),
            "[Handoff hf_7k3m9p2q4r] The user opened a request: \
             \"I'm about to create the API key on Stripe\". Produce the spec and call \
             handoff_to_user with request_id=hf_7k3m9p2q4r."
        );
    }

    #[test]
    fn the_italian_sentence_is_pinned_as_well() {
        // The clipboard renders in the user's language (OPEN-05), so the Italian sentence is
        // as much a thing an agent reads as the English one; only the design prints the
        // English. Pinned here so a translation cannot drift away from the shape the agent
        // has to act on — the quoted words, the tool name and `request_id=`.
        assert_eq!(
            render_request_text(Language::It, ID, WHAT),
            concat!(
                "[Handoff hf_7k3m9p2q4r] L'utente ha aperto una richiesta: ",
                "\"I'm about to create the API key on Stripe\". ",
                "Produci la spec e chiama handoff_to_user con request_id=hf_7k3m9p2q4r."
            )
        );
    }

    #[test]
    fn the_resume_sentence_is_pinned_in_both_languages() {
        // FM-31: what an agent reads when the user picked a handoff up in the overlay.
        assert_eq!(
            render_resume_text(Language::En, ID),
            concat!(
                "[Handoff hf_7k3m9p2q4r] The user resumed this handoff in the overlay. ",
                "Call handoff_to_user with resume=hf_7k3m9p2q4r to pick it up."
            )
        );
        assert_eq!(
            render_resume_text(Language::It, ID),
            concat!(
                "[Handoff hf_7k3m9p2q4r] L'utente ha ripreso questo handoff dal pannello. ",
                "Chiama handoff_to_user con resume=hf_7k3m9p2q4r per riprenderlo."
            )
        );
    }

    #[test]
    fn the_id_and_the_tool_name_are_the_same_in_both_languages() {
        for language in [Language::En, Language::It] {
            for rendered in [
                render_request_text(language, ID, WHAT),
                render_resume_text(language, ID),
            ] {
                assert!(rendered.contains(ID), "{language}: {rendered}");
                assert!(
                    rendered.contains("handoff_to_user"),
                    "{language}: {rendered}"
                );
                assert!(!rendered.contains('{'), "{language} left a placeholder");
            }
            // The parameter names of §4.7.1: an open quotes `request_id`, a resume `resume`.
            assert!(render_request_text(language, ID, WHAT).contains("request_id=hf_7k3m9p2q4r"));
            assert!(render_resume_text(language, ID).contains("resume=hf_7k3m9p2q4r"));
        }
    }

    #[test]
    fn the_two_languages_are_not_the_same_sentence() {
        // A key added to English and copied verbatim into Italian is the failure APP-02 is
        // about; the parity test only proves the key is there.
        assert_ne!(
            render_request_text(Language::En, ID, WHAT),
            render_request_text(Language::It, ID, WHAT)
        );
        assert_ne!(
            render_resume_text(Language::En, ID),
            render_resume_text(Language::It, ID)
        );
    }

    #[test]
    fn a_request_whose_own_words_look_like_a_placeholder_keeps_them() {
        let rendered = render_request_text(Language::En, ID, "rename {id} to {text}");
        assert!(rendered.contains("rename {id} to {text}"), "{rendered}");
    }

    #[test]
    fn a_queue_entry_is_rendered_by_what_it_is() {
        let asking_for_a_spec = request(ID);
        assert_eq!(
            render_for(Language::En, &asking_for_a_spec),
            render_request_text(Language::En, ID, &asking_for_a_spec.text)
        );

        let mut coming_back = request("hf_0000000001");
        coming_back.about_handoff_id = Some(ID.to_owned());
        coming_back.text = resume_queue_text(ID);
        assert_eq!(
            render_for(Language::En, &coming_back),
            render_resume_text(Language::En, ID)
        );
        // The sentence names the handoff to resume, never the queue entry's own id.
        assert!(!render_for(Language::En, &coming_back).contains("hf_0000000001"));
    }

    #[test]
    fn the_stored_text_of_a_resume_names_its_handoff() {
        assert_eq!(resume_queue_text(ID), "Resume handoff hf_7k3m9p2q4r");
    }
}
