package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

public class HexRenderer extends SeperatorRenderer {
    
    //FIXME: allow setting of groupings widths
    
    private int[] hexStart; // cached/lazy
    private int base;
    private boolean zeropad = true; // pad short numbers with 0's
    private boolean showPrintables = false;
    private boolean showSpecials = false;  //fixme: should have a seperate option whether to show \a (0x07) and \v (0x0b)

    private String specials = "\b\f\n\r\t\0\007\013";
    
    
    public HexRenderer(FontMetrics fm, int columnCount, boolean render, int base, boolean zeropad,  boolean showPrintables, boolean showSpecials) {
        super(fm,columnCount,render);
        setBase(base);
        setZeropad(zeropad);
	setShowPrintables(showPrintables);
	setShowSpecials(showSpecials);
    }
    
    public HexRenderer(FontMetrics fm, int columnCount, boolean render, int base, boolean zeropad) {
	this(fm,columnCount,render,base,zeropad,false,false);
    }
        
    public HexRenderer(FontMetrics fm, int columnCount, boolean render) {
        this(fm,columnCount,render,16,true,false,false);
    }
    
    public void setBase(int base) {
        this.base = base;
    }
    
    public void setZeropad(boolean zeropad) {
        this.zeropad = zeropad;
    }

    public void setShowPrintables(boolean showPrintables) {
        this.showPrintables = showPrintables;
    }
    
    public void setShowSpecials(boolean showSpecials) {
        this.showSpecials = showSpecials;
    }
    
    // longest length for a byte "symbol"
    private int maxChars() {
        int unsigned = 0xff; // highest value
        return Integer.toString(unsigned, base).length();
    }

    //FIXME: make this a static lookup
    private String byte2hex(byte b) {
        int unsigned = ((int)b & 0xff);
	boolean dozeropad; 
	
        String hex;

	// note: code kinda duplicated in AsciiRenderer
	if (showPrintables && (unsigned >= 0x20 && unsigned < 0x7F)) {
	    // printable character
	    hex = (char)unsigned + "";
	    dozeropad = false;
	} else if (showSpecials && specials.indexOf(unsigned) != -1) {
	    if (unsigned=='\b') hex = "\\b";
	    else if (unsigned=='\f') hex = "\\f";
	    else if (unsigned=='\n') hex = "\\n";
	    else if (unsigned=='\r') hex = "\\r";
	    else if (unsigned=='\t') hex = "\\t";
	    else if (unsigned=='\0') hex = "\\0";
	    else if (unsigned=='\007') hex = "\\a"; // note: \a is not in java
	    else if (unsigned=='\013') hex = "\\v"; // 0x0b, note: \v is not in java
	    else hex = "??"; // error
	    dozeropad = false;
	} else {
	    //usual case
	    hex = Integer.toString(unsigned, base); 
	    dozeropad = zeropad;
	}
	
        
        int padlen = maxChars() - hex.length(); 
        for (int i=0; i<padlen; i++){
            if (dozeropad) {
                hex = "0"+hex;
            } else {
                hex = " "+hex;
            }
        }
        
	
        return hex;
    }
        
//    private static String byte2hex(byte b)  {
//        int l = ((int)b & 0xf0) >> 4;
//        int r = (int)b & 0x0f;
//        
//        return ""
//        	+ (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
//        	+ (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
//    }
    
    public int[] hexStart() {
        if (hexStart == null) {
            int charWidth = fm.charWidth('W');
	        int[] spaceEvery = new int[] {1, 2, 4, 8};
	        int[] spaceSize = new int[]  {charWidth*(maxChars()+1), 0, charWidth, charWidth };
	        
	        hexStart = new int[columnCount+1]; // where each hex starts on a line + where ascii starts
	        
	        int s = 0;
	        for (int n=0; n<hexStart.length; n++) { // only place hexStart.length should be used. use hexPerLine instead.
	            hexStart[n] = s;
	            for (int m=0; m<spaceEvery.length; m++)  {
	                if ((n+1) % spaceEvery[m] == 0)  {
	                    s += spaceSize[m];
	                }
	            }
	        }
        }
        
        return hexStart;
    }
    public void renderBytes( Graphics g, long lineNumber,
            				byte ba[], int baOffset, int baLength, boolean selecta[],
            				Place cursor) {
        //// draw hex:

        int charsHeight = fm.getHeight();
        int charsWidth = fm.charWidth('W') * maxChars();
        int[] hexStart = hexStart();
        
        for ( int i=0; i < baLength; i++ ) {
            if (selecta[i+baOffset])  {
                g.setColor( new Color(170,170,255) );
                
                // fill in gaps between text e.g. "FF (here) FF"
                int fillWidth = (i+1 < baLength && selecta[i+baOffset+1]) ? hexStart[i+1] - hexStart[i] : charsWidth; 
                g.fillRect( hexStart[i], 0, fillWidth, charsHeight);
            }
            
            g.setColor(Color.black);
            g.drawString( byte2hex(ba[baOffset+i]), hexStart[i], fm.getAscent());
            
        }
    }
    
    

    public int getWidth() {
        int charsWidth = fm.charWidth('W') * maxChars();
        int[] hexStart = hexStart();
        
        return hexStart[columnCount-1] + charsWidth;
    }
    
    public void setColumnCount(int cc) {
        hexStart = null; // reset for recalc
        super.setColumnCount(cc);
    }
    public void setFontMetrics(FontMetrics fm) {
        hexStart = null; // reset for recalc
        super.setFontMetrics(fm);
    }
    
    public long whereAmI(int x, int y, long lineAddress){
        int[] hs = hexStart();
        for (int i = 0; i < hs.length; i++) {
            int start = hs[i];
            if (x < start)
                return lineAddress+i-1;
        }
        return -1;
    }
    
    public String toString() {
	if (showPrintables && showSpecials)
	    return "character display (" + base(base) + ")";
	
	return base(base);
	
    }
    
    private static String base(int base) {
	if (base==16)
            return "Hex";
        
        if (base==10)
            return "Decimal";
        
        if (base==8)
            return "Octal";
        
        if (base==2)
            return "Binary";

        return "base " + base;

    }

}
