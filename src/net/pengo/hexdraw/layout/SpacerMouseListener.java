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
 * SpacerMouseListener.java
 *
 * Created on 30 November 2004, 00:16
 */

package net.pengo.hexdraw.layout;

import java.awt.event.MouseEvent;
import java.awt.event.MouseMotionListener;
import javax.swing.event.MouseInputListener;

import net.pengo.bitSelection.BitCursor;

import net.pengo.bitSelection.SegmentalBitSelectionModel;

import static net.pengo.hexdraw.layout.Spacer.*;

/**
 *
 * @author Peter Halasz
 */
public class SpacerMouseListener implements MouseInputListener { // == MouseListener, MouseMotionListener
    
    private Spacer spacer;
    private Spacer.Resolution currentRes;
    private Spacer.Resolution defaultRes = Spacer.Resolution.word;
    
    /** Creates a new instance of SpacerMouseListener */
    public SpacerMouseListener(Spacer spacer) {
        this.spacer = spacer;
    }
    
    public SegmentalBitSelectionModel getSelection() {
        return getSpacer().getActiveFile().getActive().getSelection();
    }
    public void mouseClicked(MouseEvent e) {}
    public void mouseEntered(MouseEvent e) {};
    public void mouseExited(MouseEvent e)  {};
    //public void mousePressed(MouseEvent e)  {};
    public void mouseReleased(MouseEvent e)  {};
    //public void mouseDragged(MouseEvent e) 
    public void mouseMoved(MouseEvent e)  {};
    
    public void mousePressed(MouseEvent e)  {
        if (e.getButton() == MouseEvent.BUTTON3) {
            // right click
            //fixme: should be different when clicking on a selection
            getSpacer().getActiveFile().getActive().liveSelection.getJMenu().getPopupMenu().show(getSpacer(), e.getX(), e.getY());
            return;
        }
        
        if (e.isShiftDown()) {
            extendSelection(e.getX(), e.getY());
            return;
        }
        
        startSelection(e.getX(), e.getY());
        
        //check for double click?
    }
    
    private void extendSelection(int x, int y) {
        BitCursor bc = getSpacer().whereIsCursor(x,y,getResolution());
        System.out.println("extend selection to: " + bc);
        getSelection().setLeadSelectionIndex(bc);
    }
    
    private void startSelection(int x, int y) {
        setResolution(getDefaultRes());
        BitCursor bc = getSpacer().whereIsCursor(x, y, getResolution());
        System.out.println("start selection: " + bc);
        getSelection().setAnchor(bc);
    }
    
    // MouseMotionListener
    public void mouseDragged(MouseEvent e)  {
        int x = e.getX();
        int y = e.getY();
        BitCursor bc = getSpacer().whereIsCursor(x, y, getResolution());
        System.out.println("mouse dragged: " + bc);

    }
   
    public Spacer getSpacer() {
        return spacer;
    }

    public void setSpacer(Spacer spacer) {
        this.spacer = spacer;
    }

    public Resolution getResolution() {
        return currentRes;
    }

    public void setResolution(Resolution currentRes) {
        this.currentRes = currentRes;
    }

    public Resolution getDefaultRes() {
        return defaultRes;
    }

    public void setDefaultRes(Resolution defaultRes) {
        this.defaultRes = defaultRes;
    }

    
}
