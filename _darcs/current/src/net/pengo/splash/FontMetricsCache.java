package net.pengo.splash;

import java.awt.*;
import java.io.InputStream;
import java.net.URL;
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
        
        //***********
        // put registrations here:
        
        registerFontToFind("hex.S", new Font("Monospaced", Font.BOLD, 8) );
        registerFontToFind("hex", new Font("Monospaced", Font.BOLD, 11) );
        registerFontToFind("hex.L", new Font("Monospaced", Font.BOLD, 14) );
        registerFontToFind("hex.XL", new Font("Monospaced", Font.BOLD, 24) );
        
        //fixme: should try: Lucida Sans Unicode, arial unicode ms, Microsoft Sans Serif, SansSerif
        registerFontToFind("unicode", new Font("Lucida Sans Unicode",Font.PLAIN,11));
        
        
        registerEcclemony(); // PreEcclenony Regular, or Futurama Alien Alphabet One
        //registerFontToFind("futurama", new Font("Futurama Alien Alphabet One", Font.PLAIN, 12) );
        
        //***********
        
    }
    
    private void registerEcclemony() {
        //String futuramaFile = "net/pengo/noncode/fr-fal1.ttf";
        //String futuramaFile = "net/pengo/noncode/fr-title.ttf";
        //String futuramaFile = "net/pengo/noncode/Bin0011.ttf";
        String futuramaFile = "net/pengo/noncode/PrEcclem.ttf"; // PreEcclenony Regular
        //String futuramaFile = "net/pengo/noncode/falsepos.ttf"; // False Positive non commercial use
        //String futuramaFile = "net/pengo/noncode/reasonsh.ttf"; // non commercial use
        //String futuramaFile = "net/pengo/noncode/reason.ttf"; // non commercial use
        //String futuramaFile = "net/pengo/noncode/sequence.ttf"; // non commercial use
        //String futuramaFile = "net/pengo/noncode/zurklezo.ttf";
        //String futuramaFontName = "Futurama Alien Alphabet One"; // hmm.. not needed
        int size = 16;
        
        try {
            URL url = ClassLoader.getSystemResource(futuramaFile);
            InputStream futureStream = url.openStream();
            Font futurama = Font.createFont(Font.TRUETYPE_FONT, futureStream);
            futureStream.close();
            Font futurama12 = futurama.deriveFont((float)size);
            registerFontToFind("alien", futurama12);
        } catch (Exception e) {
            //IOException or FontFormatException 
            //fall back to monospaced.
            System.out.println("Alien font not found.");
            registerFontToFind("alien", new Font("Monospaced", Font.PLAIN, size));
        };
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
        //System.out.println("all found:" + allFound);
    }
    
    /** register a font you will later need the font metrics for.
      just add a call to this to the private constructor above. */
    public void registerFontToFind(String name, Font f) {
        fontNameMap.put(name, f);
        
        if (fontMap.get(f) != null)
            return;
        
        fontMap.put(f, null);
        allFound = false;
        //System.out.println("added \"" + name + "\" -- " + f);
    }
    
    public FontMetrics getFontMetrics(String name) {
        //FIXME: block until font comes in?
        //System.out.println("getting metrics" + name);
        return (FontMetrics)fontMap.get((Font)fontNameMap.get(name));
    }
    
    public FontMetrics getFontMetrics(Font f) {
        //System.out.println("getting metrics" + f);
        FontMetrics fm = (FontMetrics)fontMap.get(f);
        if (fm==null)
            System.out.println("this font never registered: " + f);
            
        return fm;
    }
    
    public Font getFont(String name) {
        //System.out.println("getting font" + name);
        return (Font)fontNameMap.get(name);
    }
}

