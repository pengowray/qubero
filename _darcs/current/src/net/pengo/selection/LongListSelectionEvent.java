/*
 * Based on @(#)ListSelectionEvent.java	1.18 01/12/03
//package javax.swing.event;
 
 * LongListSelectionEvent.java

 */


package net.pengo.selection;

import java.util.EventObject;
import javax.swing.*;

import javax.swing.event.*;


//FIXME: add type of change in here too?

public class LongListSelectionEvent extends EventObject
{
    private long firstIndex;
    private long lastIndex;
    private boolean isAdjusting;

    public LongListSelectionEvent(Object source, long firstIndex, long lastIndex,
			      boolean isAdjusting)
    {
	super(source);
	this.firstIndex = firstIndex;
	this.lastIndex = lastIndex;
	this.isAdjusting = isAdjusting;
    }

    public long getFirstIndex() { return firstIndex; }

    public long getLastIndex() { return lastIndex; }

    public boolean getValueIsAdjusting() { return isAdjusting; }

    public String toString() {
	String properties = 
	    " source=" + getSource() +  
            " firstIndex= " + firstIndex + 
            " lastIndex= " + lastIndex + 
	    " isAdjusting= " + isAdjusting +
            " ";
        return getClass().getName() + "[" + properties + "]";
    }
}


