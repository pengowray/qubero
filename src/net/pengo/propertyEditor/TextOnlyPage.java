/**
 * TextOnlyPage.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;

import javax.swing.JLabel;

public class TextOnlyPage extends PropertyPage {
    private String name, text;
    

    public String toString() {
	return name;
    }
    
    public void save() {}
    
    
    public TextOnlyPage(String name, String text) {
	super();
	this.name = name;
	this.text = text;
	
	add(new JLabel(text));
    }
    
    
}

