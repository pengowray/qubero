package net.pengo.splash;

import javax.swing.*;
import java.awt.*;
import java.util.*;

/** because it can be difficult to find font metric properties in a hurry.
  Especially when you dont have a Graphics object*/
public class FontMetricsCache {
    protected static FontMetricsCache singleton;

    protected Map fontMap; // font -> FontMetrics
    protected Map fontNameMap; // fontName -> font
    protected boolean allFound = true; // have all metrics been located?
    
    // use singleton() instead of this constructor.
    private FontMetricsCache() {
        fontNameMap = new HashMap();
        fontMap = new HashMap();
        allFound = true;
        
        // put registrations here:
        registerFontToFind("hex", new Font("Monospaced", Font.PLAIN, 11) );
    }

    public static FontMetricsCache singleton() {
        if (singleton == null)
            singleton = new FontMetricsCache();
        
        return singleton;
    }
    
    /* generously lend this a graphics object so the empty
     items in the cache can be filled. */
    public void lendGraphics(Graphics g) {
        if (allFound)
            return;
        
        Set set = fontMap.keySet();
        for (Iterator i = set.iterator(); i.hasNext(); ) {
            Font key = (Font)i.next();
            if (fontMap.get(key) == null) {
                fontMap.put(key, g.getFontMetrics(key));
            }
        }
        allFound = true;
    }
    
    /** register a font you will later need the font metrics for.
      just add a call to this to the private constructor above. */
    public void registerFontToFind(String name, Font f) {
        fontNameMap.put(name, f);
        
        if (fontMap.get(f) != null)
            return;
        
        fontMap.put(f, null);
        allFound = false;
    }
    
    public FontMetrics getFontMetrics(String name) {
        //FIXME: block until font comes in?
        return (FontMetrics)fontMap.get((Font)fontNameMap.get(name));
    }
    
    public FontMetrics getFontMetrics(Font f) {
        return (FontMetrics)fontMap.get(f);
    }
    
    public Font getFont(String name) {
        return (Font)fontNameMap.get(name);
    }
}

