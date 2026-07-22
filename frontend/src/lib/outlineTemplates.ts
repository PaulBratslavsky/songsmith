// Genre song-outline templates (user-picked 2026-07-22, + Synthwave): section
// skeletons for the Library quick card and the Outline Builder page.
export const OUTLINE_TEMPLATES: { name: string; bpm: number; sections: [string, number][] }[] = [
  { name: "Pop", bpm: 100, sections: [["Intro", 4], ["Verse 1", 16], ["Pre-Chorus", 8], ["Chorus", 16], ["Verse 2", 16], ["Pre-Chorus 2", 8], ["Chorus 2", 16], ["Bridge", 8], ["Final Chorus", 16], ["Outro", 8]] },
  { name: "EDM / Dance", bpm: 126, sections: [["Intro", 16], ["Build 1", 16], ["Drop 1", 16], ["Breakdown", 16], ["Build 2", 16], ["Drop 2", 16], ["Outro", 16]] },
  { name: "Hip-hop / Trap", bpm: 140, sections: [["Intro", 8], ["Hook", 8], ["Verse 1", 16], ["Hook 2", 8], ["Verse 2", 16], ["Hook 3", 8], ["Outro", 8]] },
  { name: "Rock", bpm: 120, sections: [["Intro", 8], ["Verse 1", 16], ["Chorus", 8], ["Verse 2", 16], ["Chorus 2", 8], ["Solo", 8], ["Bridge", 8], ["Final Chorus", 16], ["Outro", 8]] },
  { name: "Synthwave", bpm: 105, sections: [["Intro", 16], ["Verse 1", 16], ["Build 1", 8], ["Chorus", 16], ["Verse 2", 16], ["Build 2", 8], ["Chorus 2", 16], ["Breakdown", 16], ["Final Chorus", 16], ["Outro", 16]] },
];
