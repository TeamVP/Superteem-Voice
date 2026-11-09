# Makes test audio for kws_eval with Windows text to speech: training takes of each key phrase and a test
# recording with the phrases among things that sound like them. 16 kHz mono.
#   pwsh core/examples/kws_tts.ps1 -Out <dir> -Voice 'Microsoft Zira Desktop'
param([string]$Out, [string]$Voice)
Add-Type -AssemblyName System.Speech
$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
function Say($file, $body) {
  $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
  $s.SelectVoice($Voice)
  $s.SetOutputToWaveFile((Join-Path $Out $file), $fmt)
  $s.SpeakSsml("<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-GB'><voice name='$Voice'><break time='800ms'/>$body<break time='800ms'/></voice></speak>")
  $s.Dispose()
}
function Takes($phrase) {
  $rates = @('medium','medium','fast','slow','x-fast','medium')
  $pitches = @('medium','high','medium','low','medium','x-high')
  ($rates | % -Begin { $i = 0 } -Process { "<prosody rate='$_' pitch='$($pitches[$i])'>$phrase</prosody><break time='1200ms'/>"; $i++ }) -join ''
}
New-Item -ItemType Directory -Force $Out | Out-Null
Say 'train_wake.wav' (Takes 'Hey Superteem')
Say 'train_bookmark.wav' (Takes 'Bookmark that')
Say 'train_action.wav' (Takes 'Action item')
Say 'train_stop.wav' (Takes 'Stop recording')
Say 'train_yes.wav' (Takes 'Yes')
Say 'train_no.wav' (Takes 'No')
# Ordinary talk full of syllables like "no" and "yes", with the answers said on their own twice. Only those
# two should be heard as answers.
$answers = @(
  "So, I know what you mean. Go on, though, now is not the time.",
  "No.",
  "Nobody said it was easy, and no one knows the new note's owner. Oh, so slow.",
  "Yes is what we told them; yesterday's notes say so too. Nothing, nope, never mind.",
  "Yes.",
  "Let's go over the snow report, then the notice board. Is that so? I suppose so."
) -join "<break time='900ms'/>"
Say 'test_answers.wav' $answers
# Expected hits are marked in the eval with their order: wake, bookmark, action, wake, stop, wake.
$test = @(
  "Morning everyone, thanks for joining. We have a super team this quarter and the numbers show it.",
  "Hey Superteem, bookmark that.",
  "The supplier says delivery slips to March, which is a problem for the steam test rig.",
  "OK so that is an action item for Raj: chase the supplier by Friday.",
  "Hey Steve, can you share the budget? Super. The team will review it tomorrow.",
  "<prosody rate='fast'>Hey Superteem</prosody>, what did we decide about the launch?",
  "Stop me if you have heard this, but recording the demo early saved us last time.",
  "Book a room for Thursday, and mark that on the calendar. Actions speak louder than items on a list.",
  "<prosody rate='slow'>Hey Superteem</prosody>. Stop recording."
) -join "<break time='700ms'/>"
Say 'test.wav' $test
