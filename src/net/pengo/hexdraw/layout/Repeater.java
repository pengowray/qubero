/*
 * Created on 19/01/2005
 *
 * TODO To change the template for this generated file go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
package net.pengo.hexdraw.layout;

import java.awt.Graphics;

import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.data.Data;

/**
 * @author Que
 *
 * TODO To change the template for this generated type comment go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
public class Repeater extends MultiSpacer {
	private SuperSpacer contents;
	private boolean horizontal;
	
	private long maxRepeats =-1;
	
	public Repeater() {
		
	}
	
	public Repeater(SuperSpacer contents) {
		this.contents = contents;
	}
	
    public void setContents(SuperSpacer contents) {
    	this.contents = contents;
    	//recalc();
    }

	public long getSubSpacerCount() {
		//FIXME
		return 2;
	}

	public long getPixelWidth(BitCursor bits) {
		RepeatSpacerSimple main = getMainRepeater(bits); 
		BitCursor leftover = getLeftover(bits);
		
		if (isHorizontal()) {
			return main.getPixelWidth() + contents.getPixelWidth(leftover);
		} else {
			return Math.max(main.getPixelWidth(), contents.getPixelWidth(leftover));
		}
	}

	public long getPixelHeight(BitCursor bits) {
		RepeatSpacerSimple main = getMainRepeater(bits); 
		BitCursor leftover = getLeftover(bits);
		
		if (!isHorizontal()) {
			//System.out.println("main.getPixelHeight():" + main.getPixelHeight() + " contents.getPixelHeight(leftover):" + contents.getPixelHeight(leftover)
			//		+ " contents.getPixelHeight(bits): " + contents.getPixelHeight(bits));

			return main.getPixelHeight() + contents.getPixelHeight(leftover);
			
		} else {
			return Math.max(main.getPixelHeight(), contents.getPixelHeight(leftover));
		}
	}
	
	private RepeatSpacerSimple getMainRepeater(BitCursor bits) {
		//FIXME: cache return value
		BitCursor contentBits = contents.getBitCount(bits);
		
		long repeats = bits.divide(contentBits);
		//System.out.println("getMainRepeater.. h:" +isHorizontal() + " bits:" + bits + " contentBits:" + contentBits + " repeats:" + repeats + " maxReps:" + maxRepeats);
		if (maxRepeats != -1 && repeats >= maxRepeats)
			repeats = maxRepeats;
				
		RepeatSpacerSimple main = new RepeatSpacerSimple();
		main.setContents(contents);
		main.setHorizontal(isHorizontal());
		main.setContentsBits(contentBits);
		main.setRepeats(repeats);
		
		return main;
	}
	
	private BitCursor getLeftover(BitCursor bits) {
		BitCursor contentBits = contents.getBitCount(bits);

		//if more than maxRepeats, no leftovers
		long repeats = bits.divide(contentBits);
		if (maxRepeats != -1 && repeats >= maxRepeats)
			return bits.zero;
		
		BitCursor leftover = bits.mod(contentBits);
		return leftover;
	}
	
	public BitCursor getBitCount(BitCursor bits) {
		if (maxRepeats == -1)
			return bits;
		
		return getMainRepeater(bits).getBitCount(bits).add(getLeftover(bits));
	}

	public BitCursor bitIsHere(long arg0, long arg1, net.pengo.hexdraw.layout.SuperSpacer.Round arg2, BitCursor arg3) {
		// TODO Auto-generated method stub
		return null;
	}

	public void paint(Graphics g, Data d, BitSegment seg) {
		BitCursor len = seg.getLength();
		
		//FIXME: should cut short segment for main repeater? not really needed because number of repeats is worked out
		RepeatSpacerSimple main = getMainRepeater(len);
		main.paint(g,d,seg);
		
		BitCursor leftover = getLeftover(len);
		if (leftover.equals(BitCursor.zero))
				return;
		
		long tranX = (isHorizontal() ? main.getPixelWidth() : 0);
		long tranY = (isHorizontal() ? 0 : main.getPixelHeight());
		g.translate((int)tranX, (int)tranY);
		
		BitSegment leftoverSeg = new BitSegment( seg.lastIndex.subtract(leftover), seg.lastIndex);

		contents.paint(g, d, leftoverSeg);
		
		g.translate((int)-tranX, (int)-tranY);
	}

	public boolean isHorizontal() {
		return horizontal;
	}
	public void setHorizontal(boolean horizontal) {
		this.horizontal = horizontal;
		//recalc();
	}
	public long getMaxRepeats() {
		return maxRepeats;
	}
	public void setMaxRepeats(long maxRepeats) {
		this.maxRepeats = maxRepeats;
	}
}
