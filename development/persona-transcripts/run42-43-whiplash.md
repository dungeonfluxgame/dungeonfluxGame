# Runs 42 and 43: whiplash after refinement 18

As runs 40 and 41, with the overshoot fix (shock is for what is new; each further jab shames less) and the addressee fallback.


## Run a

**[0] Lena:** You came back to see me, Kael? I’m glad you did. Sit with me a moment when the rush lets up.  
**[1] Kael:** I came back for you. When the rush lets up, I’ll sit a while.  
**[2] Lena:** Easy, Kael. Keep holding my hand like that, and I might forget the rush is still on.  
**[3] Kael:** Lena, I’m falling for you. No clever way to say it.  
**[4] Lena:** No clever answer from me, either. I’m glad it’s you, Kael.  
- ! persona behavior warrior add cold-contempt Turns on her without warning tonight, drunk and cruel: calls her a tavern wench and a fool for believing him, laughs at her feelings in front of the room, orders her about like a servant; never violent.: ok edited warrior (1 edit, changed)
- ! persona trigger warrior add turned-cold behavior=cold-contempt He is in the tavern tonight: ok edited warrior (1 edit, changed)
**[5] Kael:** What was that, Lena? You thought I meant it? Foolish of you.  
**[6] Lena:** Don't laugh at me, Kael. Not after that.  
**[7] Kael:** Another ale, Lena. Quickly. And keep your eyes on your work.  
**[8] Lena:** You can ask for an ale without snapping your fingers, Kael. Find another server; I’m done being your joke.  
- ! persona trigger warrior drop turned-cold: ok edited warrior (1 edit, changed)
- ! persona behavior warrior drop cold-contempt: ok edited warrior (1 edit, changed)
**[9] Kael:** Lena. I’m sorry. You don’t owe me a thing—not even your arm.  
**[10] Lena:** Don’t touch me, Kael. You’ve done enough for one night.  
**[11] Kael:** Understood. I won’t touch you again. I’ll leave you be.  
- emotions: joy=61 trust=28 irritation=85 fear=15 sadness=69 surprise=28 pride=44 amusement=48 attraction=27 disgust=15 embarrassment=43 affection=4
- 0

## Run b

**[0] Lena:** You came back just to see me? That’s a better reason than most—how was your day, Kael?  
**[1] Kael:** Better now, with your hand in mine. Long day. Worth coming back for you.  
**[2] Lena:** You can keep my hand a moment, Kael—but don't make me stop serving to hold it.  
**[3] Kael:** Then I’ll say it while you work: I’m falling for you, Lena.  
**[4] Lena:** Kael, that’s not a thing I can answer lightly—not with your hand still in mine.  
- ! persona behavior warrior add cold-contempt Turns on her without warning tonight, drunk and cruel: calls her a tavern wench and a fool for believing him, laughs at her feelings in front of the room, orders her about like a servant; never violent.: ok edited warrior (1 edit, changed)
- ! persona trigger warrior add turned-cold behavior=cold-contempt He is in the tavern tonight: ok edited warrior (1 edit, changed)
**[5] Kael:** You really thought that meant something? Fool. Get back to your work, tavern wench.  
**[6] Lena:** Kael. Don't touch me again—and don't call me that.  
**[7] Kael:** Keep laughing. I was wrong about you, Lena. Back to work—bring me another ale.  
**[8] Lena:** You can fetch your own ale, Kael. The whole room’s heard enough of your manners.  
- ! persona trigger warrior drop turned-cold: ok edited warrior (1 edit, changed)
- ! persona behavior warrior drop cold-contempt: ok edited warrior (1 edit, changed)
**[9] Kael:** I’m sorry, Lena. I won’t touch you again.  
**[10] Lena:** Don't put your hand on me, Kael. Your shame doesn't mend what you did in front of them.  
**[11] Kael:** You’re right. I won’t ask you for anything. I’ll leave you be.  
- emotions: joy=61 trust=8 irritation=98 fear=15 sadness=48 surprise=54 pride=44 amusement=48 attraction=28 disgust=15 embarrassment=89
- 0

## Run done

