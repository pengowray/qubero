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

/**
 * SimpleAddressPage.java
 *
 *
 *
 * @author Peter Halasz
 */

package net.pengo.propertyEditor;

import javax.swing.JTextField;

import net.pengo.app.OpenFile;
import net.pengo.data.SelectionData;
import net.pengo.resource.AddressedResource;
import net.pengo.resource.DefaultSelectionResource;
import net.pengo.resource.SelectionResource;
import net.pengo.selection.SimpleLongListSelectionModel;

public class SimpleAddressPage extends EditablePage {
    private AddressedResource res;
    private JTextField addressField;
    private JTextField lengthField;
    
    public SimpleAddressPage(AddressedResource res, PropertiesForm form) {
        super(form);
        this.res = res;
        
        addressField = new JTextField("0",12);
        addressField.addActionListener( getSaveActionListener() );
        addressField.getDocument().addDocumentListener( this );
        
        lengthField = new JTextField("0",12);
        lengthField.addActionListener( getSaveActionListener() );
        lengthField.getDocument().addDocumentListener( this );
        
        add(addressField);
        add(lengthField);
        
        build();
    }
    
    public void buildOp() {
        SelectionResource sel = res.getSelectionResource();
        SelectionData selData = sel.getSelectionData();
        
        addressField.setText(selData.getStart()+"");
        lengthField.setText(selData.getLength()+"");
    }
    
    
    protected void saveOp() {
        long start = Long.parseLong( addressField.getText());
        long end = start + Long.parseLong( lengthField.getText()) - 1;
        OpenFile of = res.getSelectionResource().getOpenFile();
        res.setSelectionResource(new DefaultSelectionResource(of, new SimpleLongListSelectionModel(start, end)));
    }
    
    
    public String toString() {
        return "Simple address";
    }
    
}

