/*
 * Created on Feb 20, 2005
 */
package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.text.Format;

import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.SegmentalBitSelectionModel;
import net.pengo.data.Data;
import net.pengo.hexdraw.layout.SuperSpacer.Round;
import net.pengo.splash.SimpleSize;
import net.pengo.splash.SimplySizedFont;

/**
 * @author Peter Halasz
 */
public class Address extends SingleSpacer {

	private SimplySizedFont font;
	private BitCursor bitSpan;
	
	//FIXME: getPixelWidth needs more information. not just the bits, but the length of the entire file
	//so that the minimum character length of the address field can be calculated.
	public long getPixelWidth(BitCursor bits) {

		if (bits.isZero()) {
			//FIXME: this is needed, but shouldn't be. Hunt down caller.
			return 0;
		}
		
		int maxLen = "0000:0000.0".length();
		int x = font.getFontMetrics().stringWidth("0000:0000.0"); // gives doubled result 
		//int x = font.getFontMetrics().charWidth('0') * maxLen; 
		return x;
	}

	public long getPixelHeight(BitCursor bits) {
		return font.getFontMetrics().getHeight();
	}
	
	/** number of bits to use. usually bitSpan. */
	private BitCursor bits(BitCursor bits) {
		if (bitSpan == null)
			return bits;
		
		if (bits.compareTo(bitSpan) < 0)
			return bits;
		
		return bitSpan;
	}

	public BitCursor getBitCount(BitCursor bits) {
		return bits(bits);
	}

	public void updateLayoutCursor(LayoutCursorBuilder lc,
			LayoutCursorDescender curs) {
		// TODO Auto-generated method stub

	}

	public void paint(Graphics g, Data d, BitSegment seg,
			SegmentalBitSelectionModel sel, LayoutCursorDescender curs,
			BitSegment repaintSeg) {

		long addr = seg.firstIndex.getByteOffset();
		int bit = seg.firstIndex.getBitOffset();

		String loc = zeroPad(Long.toHexString(addr), 8, true);
		loc = insertColon(loc, 4);
		if (bit != 0) {
			loc += "." + bit; 
		} else {
			//loc += "  ";
		}
		
		g.setFont(font.getFont());
		g.drawString(loc, 0, font.getFontMetrics().getAscent());
	}

	private String insertColon(String loc, int i) {
		return loc.substring(0,i) + ":" + loc.substring(i);
	}

	public String zeroPad(String hex, int maxChars, boolean dozeropad) {
        int padlen = maxChars - hex.length(); 
        for (int i=0; i<padlen; i++){
            if (dozeropad) {
                hex = "0"+hex;
            } else {
                hex = " "+hex;
            }
        }

        return hex;
	}
	
	public void setSimpleSize(SimpleSize s) {
		if (font == null)
			return;
		
		System.out.println("resizing:" + font);
		font = font.resize(s);
	}

	public BitCursor getBitSpan() {
		return bitSpan;
	}
	public void setBitSpan(BitCursor bitSpan) {
		this.bitSpan = bitSpan;
	}
	public SimplySizedFont getFont() {
		return font;
	}
	public void setFont(SimplySizedFont font) {
		this.font = font;
	}
	
    protected void bitIsHere(LayoutCursorBuilder lc, Round round) {
    	lc.setNull(true);
    }
	
}
