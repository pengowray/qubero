/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

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


