"""PII redactor — masks emails, phone numbers, IBANs, credit cards, IPs.

Conservative pre-LLM redaction : we mask in the *user-facing* prompt
before it ever reaches the model. This is one (cheap, imperfect)
defense layer ; the M21 LLMRouter has its own DLP pass.
"""
from __future__ import annotations

import re

_EMAIL = re.compile(r"\b[\w._%+-]+@[\w.-]+\.[A-Za-z]{2,}\b")
_PHONE = re.compile(r"\b(?:\+?\d{1,3}[\s.-]?)?(?:\(?\d{2,4}\)?[\s.-]?){2,4}\d{2,4}\b")
_IBAN = re.compile(r"\b[A-Z]{2}\d{2}[\s\dA-Z]{12,30}\b")
_CC = re.compile(r"\b(?:\d[ -]?){13,19}\b")
_IPV4 = re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")


def redact_pii(text: str) -> str:
    if not text:
        return text
    text = _EMAIL.sub("[REDACTED_EMAIL]", text)
    text = _IBAN.sub("[REDACTED_IBAN]", text)
    text = _CC.sub("[REDACTED_CC]", text)
    # Phone last to avoid eating IBAN digits.
    text = _PHONE.sub("[REDACTED_PHONE]", text)
    text = _IPV4.sub("[REDACTED_IP]", text)
    return text
