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
 * SummaryPage.java
 *
 * Created on July 9, 2003, 11:26 PM
 */

package net.pengo.propertyEditor;

import javax.swing.JTextPane;

import net.pengo.resource.IntAddressedResource;

/**
 *
 * @author  Peter Halasz
 */
public class SummaryPage extends PropertyPage {
    private IntAddressedResource res;
    
    /** Creates a new instance of SummaryPage */
    public SummaryPage(IntAddressedResource res) {
        super();
        this.res = res;
        build();
    }
    
    public void build() {
        removeAll();
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        //add(new JLabel( "Size (bytes): " + res.getSelectionData().getLength() ));
        
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        JTextPane jtp = new JTextPane();
        String s = "Value: " + res.getValue() + "\n";
        s = s  + "Size (bytes): " + res.getSelectionResource().getSelectionData().getLength();
        jtp.setText(s ) ;
        add(jtp);
    }
    
    public void save() {
        return;
    }
    
    public String toString() {
        return "Summary";
    }
}
