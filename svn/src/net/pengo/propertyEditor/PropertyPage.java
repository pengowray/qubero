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
 * PropertyPage.java
 *
 * Created on July 9, 2003, 11:23 PM
 */

package net.pengo.propertyEditor;


import javax.swing.JPanel;
/**
 *
 * @author  Peter Halasz
 */
public abstract class PropertyPage extends JPanel {
    //fixme: maybe have a standard template layout someday?
    //fixme: make into a resource listener!
    

    /** Creates a new instance of PropertyPage */
    public PropertyPage() {
        super();
    }
    
    // save changes (apply/okay)
    abstract public void save();
    
    // refresh the page after save changes
    public void build() {
        return;
    }
    
    // call when the page is modified, so it knows it should be saved.
    // can also be called if the page is selected by MethodSelectionPage
    public void mod() {
        return;
    }

	// checks that properties are OK and ready to be sxtracted.
    public boolean isValid() {
        return true; // success
    }

    
    abstract public String toString();
    
    public void setForm(PropertiesForm form){}


}
