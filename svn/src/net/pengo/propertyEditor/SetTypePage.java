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
import javax.swing.JComboBox;
import javax.swing.JLabel;

import net.pengo.resource.IntAddressedResource;

/**
 *
 * @author  Peter Halasz
 */
public class SetTypePage extends EditablePage {
    private IntAddressedResource  res;
    private JComboBox type;
    
    /** Creates a new instance of SummaryPage */
    public SetTypePage(IntAddressedResource res, PropertiesForm form) {
        super(form);
        this.res = res;

        add(new JLabel( "Integer Type: " ));
        type = new JComboBox(new String[] {
            "unsigned", "ones complement (+/-0)", "twos complement (default)", "sign magnitude", "unused sign bit (NYI)"} );
        add(type);
        type.addActionListener( getModActionListener() );
        
        build();
    }
    
    public void buildOp() {
        type.setSelectedIndex( res.getSigned() );
    }
    
    public void saveOp() {
        res.setSigned( type.getSelectedIndex() );
        //System.out.println("setting sign to: " + type.getSelectedIndex());
    }
    
    public String toString() {
        return "Type";
    }
}
